// The shared building blocks. Modules draw with these, so everything in the window
// looks like one app.
export { type DiffAddon, DiffStat, DiffView, scrollToFile } from "./DiffView";
export { Markdown } from "./Markdown";
export { CommandTrace, DataTrace, ItemTrace, ReasoningTrace, ToolTrace } from "./AgentTrace";
export { StatusMark } from "./StatusMark";
export { Duration } from "./Duration";
export { Choice, type ChoiceOption } from "./Choice";
export { InteractionOutcome, InteractionPrompt } from "./InteractionPrompt";
export { type NodeAction, NodeView } from "./NodeView";
export { EffortPicker } from "./EffortPicker";
export { ModelPicker } from "./ModelPicker";
export { ThreadMenu } from "./ThreadMenu";
export { EnvEditor } from "./EnvEditor";
export { ResizeHandle } from "./ResizeHandle";
export { Remembered, useRemembered } from "./Remembered";
export { AttachmentTile, MessageAttachments, PictureDialog } from "./Attachments";
export { KeepInPlace, type Place, useKeepInPlace } from "./KeepInPlace";
export { ProjectIconView, projectIconLabel } from "./ProjectIcon";
export { CopyButton } from "./CopyButton";
export { ItemMenu } from "./ItemMenu";
