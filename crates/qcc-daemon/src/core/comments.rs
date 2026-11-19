//! Comments the user leaves on lines of a thread's changes, and how its agent
//! learns of them.

use qcc_domain::{CommentAuthor, CommentId, LineComment, ThreadId};
use qcc_protocol::jsonrpc::{RpcError, codes};
use qcc_protocol::methods::{
    COMMENTS_UPDATED, CommentAddParams, CommentDeleteParams, CommentListParams, CommentListResult,
    CommentResolveParams, CommentResult, CommentsUpdatedNotification, Empty,
};

use qcc_plugin_host::PluginRequest;

use super::{Core, notify, now_ms, thread_not_found};

fn internal(error: qcc_store::StoreError) -> RpcError {
    RpcError::new(codes::INTERNAL_ERROR, error.to_string())
}

impl Core {
    pub(super) fn comment_list(&self, p: CommentListParams) -> Result<CommentListResult, RpcError> {
        self.supervisor.thread(&p.thread_id).ok_or_else(|| thread_not_found(&p.thread_id))?;
        let mut comments = self.store.comments(&p.thread_id).map_err(internal)?;
        if !p.include_resolved {
            comments.retain(LineComment::is_open);
        }
        Ok(CommentListResult { comments })
    }

    pub(super) fn comment_add(&mut self, p: CommentAddParams) -> Result<CommentResult, RpcError> {
        self.add_comment(p, None).map(|comment| CommentResult { comment })
    }

    /// Leave a comment: the user's, or a plugin's with `author` saying whose.
    pub(super) fn add_comment(
        &mut self,
        p: CommentAddParams,
        author: Option<CommentAuthor>,
    ) -> Result<LineComment, RpcError> {
        self.supervisor.thread(&p.thread_id).ok_or_else(|| thread_not_found(&p.thread_id))?;
        let invalid = |message: &str| RpcError::new(codes::INVALID_PARAMS, message);
        let text = p.text.trim();
        if text.is_empty() {
            return Err(invalid("a comment needs some text"));
        }
        if p.path.trim().is_empty() || p.line == 0 {
            return Err(invalid("a comment needs a file and a line number from 1"));
        }
        let comment = LineComment {
            id: CommentId(format!("cmt_{}", uuid::Uuid::now_v7().simple())),
            thread_id: p.thread_id,
            path: p.path,
            line: p.line,
            side: p.side,
            quote: p.quote,
            text: text.to_owned(),
            created_at: now_ms(),
            resolved_at: None,
            resolution: None,
            sent_at: None,
            author,
        };
        self.store.save_comment(&comment).map_err(internal)?;
        self.comments_changed(&comment.thread_id);
        Ok(comment)
    }

    /// The thread's comments, oldest first, for a plugin.
    pub(super) fn comments_of(
        &self,
        thread: &ThreadId,
        include_resolved: bool,
    ) -> Result<Vec<LineComment>, RpcError> {
        self.comment_list(CommentListParams { thread_id: thread.clone(), include_resolved })
            .map(|result| result.comments)
    }

    pub(super) fn comment_resolve(
        &mut self,
        p: CommentResolveParams,
    ) -> Result<CommentListResult, RpcError> {
        // All found before any is changed, so a wrong id changes nothing.
        let mut comments = Vec::with_capacity(p.comment_ids.len());
        for id in &p.comment_ids {
            comments.push(self.find_comment(id)?);
        }
        let resolution = p.resolution.map(|text| text.trim().to_owned()).filter(|t| !t.is_empty());
        let now = now_ms();
        let mut threads: Vec<ThreadId> = Vec::new();
        for comment in &mut comments {
            if p.reopen {
                comment.resolved_at = None;
                comment.resolution = None;
            } else {
                comment.resolved_at = Some(now);
                comment.resolution.clone_from(&resolution);
            }
            self.store.save_comment(comment).map_err(internal)?;
            if !threads.contains(&comment.thread_id) {
                threads.push(comment.thread_id.clone());
            }
        }
        for thread in &threads {
            self.comments_changed(thread);
        }
        Ok(CommentListResult { comments })
    }

    pub(super) fn comment_delete(&mut self, p: CommentDeleteParams) -> Result<Empty, RpcError> {
        let comment = self.find_comment(&p.comment_id)?;
        self.store.delete_comment(&comment.id).map_err(internal)?;
        self.comments_changed(&comment.thread_id);
        Ok(Empty {})
    }

    fn find_comment(&self, id: &CommentId) -> Result<LineComment, RpcError> {
        self.store.comment(id).map_err(internal)?.ok_or_else(|| {
            RpcError::new(codes::INVALID_PARAMS, format!("comment {id} does not exist"))
        })
    }

    /// Note that comments were handed to the agent with a message.
    pub(super) fn comments_sent(&mut self, ids: &[CommentId]) {
        let now = now_ms();
        let mut threads: Vec<ThreadId> = Vec::new();
        for id in ids {
            let Ok(Some(mut comment)) = self.store.comment(id) else { continue };
            comment.sent_at = Some(now);
            if let Err(error) = self.store.save_comment(&comment) {
                tracing::warn!(%id, %error, "cannot note that a comment was sent");
                continue;
            }
            if !threads.contains(&comment.thread_id) {
                threads.push(comment.thread_id);
            }
        }
        for thread in &threads {
            self.comments_changed(thread);
        }
    }

    /// A plugin resolves or reopens comments it left; those of others are refused.
    pub(super) fn plugin_resolve_comments(
        &mut self,
        plugin: &str,
        ids: Vec<CommentId>,
        resolution: Option<String>,
        reopen: bool,
    ) -> Result<(), RpcError> {
        for id in &ids {
            let comment = self.find_comment(id)?;
            if comment.author.as_ref().is_none_or(|author| author.plugin != plugin) {
                return Err(RpcError::new(
                    codes::INVALID_PARAMS,
                    format!("comment {id} was not left by plugin {plugin}"),
                ));
            }
        }
        self.comment_resolve(CommentResolveParams { comment_ids: ids, resolution, reopen })
            .map(|_| ())
    }

    /// Tell every connection that watches threads what the comments on `thread` are
    /// now, and the plugins that read comments that they changed.
    fn comments_changed(&mut self, thread: &ThreadId) {
        for plugin in self.host.plugins().iter().filter(|p| p.permissions().comments) {
            let _ = plugin.post(PluginRequest::CommentsChanged(thread.clone()));
        }
        if self.watchers.is_empty() {
            return;
        }
        let comments = match self.store.comments(thread) {
            Ok(comments) => comments,
            Err(error) => {
                tracing::warn!(%thread, %error, "cannot read the comments to announce them");
                return;
            }
        };
        self.watchers.retain(|watcher| {
            let params = CommentsUpdatedNotification {
                subscription: watcher.subscription,
                thread_id: thread.clone(),
                comments: comments.clone(),
            };
            notify(&watcher.out, COMMENTS_UPDATED, params)
        });
    }

    /// The note that hands comments to an agent: each in full, and how to say it is
    /// dealt with. Comments that are gone or resolved are left out.
    pub(super) fn comments_note(&self, ids: &[CommentId]) -> String {
        let comments: Vec<LineComment> = ids
            .iter()
            .filter_map(|id| self.store.comment(id).ok().flatten())
            .filter(LineComment::is_open)
            .collect();
        if comments.is_empty() {
            return "The comments the user pointed to are already resolved or deleted.".to_owned();
        }
        let reviewed = comments.iter().any(|comment| comment.author.is_some());
        let mut note = String::from(if reviewed {
            "Comments were left on lines of the changes, by the user or by reviewers working \
             for the user (each says whose it is). "
        } else {
            "The user left comments on lines of the changes. "
        });
        note.push_str(
            "Deal with each one; when it is \
             done, mark it resolved with `qcc comment resolve <id> --note \"<what you did>\"` \
             (if `qcc` is not found, use \"$QCC_BIN\"). `qcc comment list` shows the open ones \
             again.\n",
        );
        for comment in &comments {
            note.push_str(&format!("\n- {} at {}", comment.id, comment.location()));
            if let Some(author) = &comment.author {
                note.push_str(&format!(", from {}", author.name));
                if let Some(severity) = &author.severity {
                    note.push_str(&format!(" ({severity})"));
                }
                if author.stale {
                    note.push_str(", made on an older version of the changes");
                }
            }
            if let Some(quote) = comment.quote.as_deref().filter(|quote| !quote.trim().is_empty()) {
                note.push_str(&format!("\n  > {}", quote.trim_end()));
            }
            for line in comment.text.lines() {
                note.push_str(&format!("\n  {line}"));
            }
        }
        note
    }
}
