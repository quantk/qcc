# qcc

Центр управления кодовыми агентами. Своего агентского цикла нет: работают Claude Code, Codex и
opencode, а qcc ведёт параллельные сессии, хранит их ленту и расширяется плагинами.
Архитектура и этапы — в [PLAN.md](PLAN.md).

## Состояние

Готовы этапы 0–5 и основная часть этапа 6: домен и сборщик дельт, SQLite-журнал, хост
WASM-плагинов с правами, SDK, демон `qccd`, CLI `qcc`, провайдеры реальных агентов, изоляция тредов
в git worktree и desktop-приложение (пульт управления агентами) с декларативным UI плагинов.
Впереди: веб-бандлы плагинов в iframe и оркестрация (этап 7).

| Провайдер     | Агент                         | Проверено на версии |
| ------------- | ----------------------------- | ------------------- |
| `claude-code` | `claude -p`, stream-json      | Claude Code 2.1.325 |
| `opencode`    | `opencode acp` (общий ACP-мост) | opencode 1.19.4   |
| `codex`       | `codex app-server`            | codex-cli 0.166.0   |
| `echo`        | тестовый, пример плагина      | —                   |

### Режим разрешений

У треда есть режим, который qcc передаёт агенту: `--permission-mode` при `thread spawn`,
позже — `qcc thread mode <id> <режим>`.

| Режим          | Что значит                                                        |
| -------------- | ----------------------------------------------------------------- |
| *(не задан)*   | решает собственная конфигурация агента, qcc ничего не меняет      |
| `ask`          | всё, что агент хочет согласовать, спрашивается у пользователя     |
| `accept-edits` | правки файлов в рабочем каталоге проходят сами, остальное — вопрос |
| `auto`         | решает автоматический ревьюер самого провайдера, пользователя не спрашивают |
| `full`         | ничего не спрашивается, песочницы нет                             |

Как режим доходит до агента: Claude Code — флаг `--permission-mode`; Codex — песочница, политика
одобрений и ревьюер (`user` или `auto_review`) в `thread/start`; ACP-агенты — мост сам отвечает на
запросы разрешений (поэтому `auto` для opencode недоступен: своего ревьюера у ACP нет).
Провайдер объявляет поддерживаемые режимы, см. `qcc plugin list`. Запросы, адресованные
пользователю, приходят в ленту и закрываются через `qcc interaction resolve`.

### Сообщение во время хода

`qcc thread send` в занятый тред по умолчанию ставит сообщение **в очередь**: оно уйдёт отдельным
ходом, когда текущий закончится. С `--steer` сообщение **подмешивается в идущий ход** (Claude Code,
Codex); если провайдер так не умеет (opencode), оно встаёт в очередь. Очередь хранится в журнале и
переживает рестарт демона; `qcc thread stop` её сбрасывает.

### Worktree

`qcc thread spawn --env git-worktree` даёт треду собственную рабочую копию: `git worktree` на
новой ветке `qcc/<thread-id>` от текущего `HEAD` (или от `--base <ref>`). Треды работают
параллельно и не трогают ни основной checkout, ни друг друга. Агент не запускается, пока каталог
не готов; если подготовка не удалась, тред падает, а не уходит работать в исходный каталог.

```sh
qcc thread spawn --provider claude-code --env git-worktree --prompt '…' --follow
qcc thread changes <thread-id>     # какие файлы изменены относительно базы
qcc thread diff <thread-id>        # всё одним патчем: коммиты, незакоммиченное и новые файлы
qcc thread archive <thread-id>     # отпустить сессию, удалить worktree, убрать тред из списка
```

- Каталог: `<data>/environments/git-worktree/<thread-id>/<имя-репозитория>`. Запущенный из
  подкаталога тред работает в том же подкаталоге своей копии.
- `.worktreeinclude` в корне репозитория (синтаксис gitignore) перечисляет неотслеживаемые файлы
  основного checkout, которые копируются в новый worktree: `.env`, сертификаты.
- `.qcc-env-setup.sh` выполняется в новом worktree до старта агента (`QCC_SOURCE_DIR`,
  `QCC_THREAD_ID`; лимит 15 минут). Ненулевой код отменяет создание: worktree и ветка удаляются.
- `.qcc-env-teardown.sh` выполняется перед удалением; его ошибка удалению не мешает.
- `thread archive` отказывает, если в worktree есть незакоммиченные изменения; `--force` удаляет
  всё равно. Ветка удаляется, только если на ней нет коммитов, которых больше нигде нет.
  Лента архивного треда остаётся читаемой (`qcc thread list --archived`).

Всё это делает обычный плагин `plugins/env-git-worktree` с правами на `git`, `bash` и `rmdir`;
ядро про git не знает.

## Desktop

```sh
./dev desktop   # собрать и открыть приложение; демон поднимется сам
```

Приложение — пульт управления: видно всех агентов сразу и можно ответить каждому.

- **Обзор.** Три колонки: кто ждёт вас (вопрос, запрос разрешения, сбой), кто работает (чем занят
  и сколько времени идёт ход), кто закончил (последняя реплика). На запрос разрешения можно
  ответить прямо с карточки.
- **Тред.** Лента с потоковым выводом, командами, диффами правок и запросами разрешений; композер
  с очередью и steer; смена режима разрешений; архивация с подтверждением, если в worktree есть
  незакоммиченная работа.
- **Изменения.** Для треда в worktree — список изменённых файлов и общий дифф.
- **Новый тред** (`Ctrl+N`): агент, каталог, worktree и база, модель, режим разрешений.
  `Ctrl+K` — палитра команд, `Ctrl+0` — обзор.

Бэкенд на Tauri — тонкий прокси: держит одно соединение с `qccd`, передаёт запросы и уведомления
как есть и сам запускает демон, если тот не запущен. Демон живёт дольше окна: агенты продолжают
работать, когда приложение закрыто. Типы TypeScript генерируются из `qcc-protocol`
(`apps/desktop/src/api/protocol.gen.ts`); тест падает, если файл устарел:

```sh
QCC_UPDATE_TS=1 cargo test -p qcc-protocol --test typescript   # перегенерировать
nix develop .#desktop -c ./scripts/check-desktop.sh            # clippy, tsc, vitest, сборка
```

Интерфейс модульный: всё, кроме каркаса окна, — вклады в реестр точек расширения. Как добавить
экран, вкладку треда, рендер элемента ленты или команду — в
[apps/desktop/README.md](apps/desktop/README.md).

## Быстрый старт

Одна команда, из любого шелла (сама заходит в `nix develop`):

```sh
./dev demo      # поднять демон, прогнать несколько echo-тредов, остановить
./dev           # qccd на переднем плане, данные в ./.dev
./dev qcc thread spawn --provider echo --prompt 'hello' --follow   # CLI к этому демону
./dev qcc thread spawn --provider claude-code --permission-mode ask --prompt 'что в этом каталоге?' --follow
./dev qcc thread send <thread-id> 'и ещё вот что' --steer
./dev desktop   # desktop-приложение (nix develop .#desktop)
./dev check     # fmt, clippy -D warnings, все тесты
./dev clean     # удалить ./.dev
```

Внутри `nix develop` то же доступно как `make run|demo|check|test|clean`.

У каждой команды `qcc` есть `--json`. Вне `./dev` данные лежат в `~/.local/share/qcc`
(`QCC_DATA_DIR`), сокет — в `$XDG_RUNTIME_DIR/qcc/qccd.sock` (`QCC_SOCKET`).

## Устройство

```
wit/plugin.wit            контракт плагинов (qcc:plugin@0.1.0)
crates/qcc-domain         типы без IO: грамматика дельт и каноническая лента
crates/qcc-runtime        сборщик дельт, автомат треда, восстановление после рестарта
crates/qcc-store          SQLite: журнал событий и проекции
crates/qcc-plugin-host    wasmtime, права, актор плагина
crates/qcc-plugin-sdk     гостевая сторона контракта
crates/qcc-plugin-testkit конформанс-набор для мостов
crates/qcc-plugins-bundle сборка plugins/ в wasm-компоненты при сборке демона
crates/qcc-protocol       API демона (JSON-RPC по unix-сокету)
crates/qcc-client         Rust-клиент API
crates/qcc-daemon         qccd
crates/qcc-cli            qcc
apps/desktop              пульт: src-tauri (прокси к qccd) + React
plugins/provider-echo     пример провайдера
plugins/provider-claude-code, provider-acp, provider-codex   мосты к реальным агентам
plugins/env-git-worktree  окружение: git worktree на каждый тред
```

Главное правило: **мост знает диалект, ядро знает ленту.** Плагин-провайдер переводит вывод
агента в узкий набор дельт; id, порядок и жизненный цикл ходов и элементов принадлежат сборщику
в `qcc-runtime`. Ядро нигде не ветвится по id провайдера, а встроенные плагины идут тем же путём,
что и сторонние.

Ни один импорт хоста не блокирует, поэтому работа, которая ждёт процесс, отвечает позже: `handle`
возвращает `deferred(token)`, а результат приходит через `completions.complete`. Так устроен
`env-git-worktree`: каждая операция — автомат над завершившимися командами git
(`src/jobs.rs`, тестируется без wasm).

## Мосты и записанный трафик

Форматы протоколов агентов не берутся по памяти: в `plugins/provider-*/fixtures/*.ndjson` лежит
трафик, записанный с установленных CLI, а рядом — `*.timeline.jsonl`, лента, в которую он обязан
собраться. Тесты `plugins/provider-*/tests/conformance.rs` проигрывают запись вместо процесса
агента через настоящий wasm-хост и сборщик.

Когда агент обновился и поведение изменилось:

```sh
# записать сценарий заново (скрипты запускают настоящий CLI)
python3 scripts/fixtures/record-claude.py RAW.ndjson allow '[{"send": …}, {"wait": "result"}]'
python3 scripts/fixtures/record-jsonrpc.py RAW.ndjson '[…]' -- opencode acp
# убрать пути, учётные данные и лишнее; скрипт проверяет, что ничего личного не осталось
python3 scripts/fixtures/redact.py claude RAW.ndjson plugins/provider-claude-code/fixtures/NAME.ndjson --work "$PWD"
# перезаписать эталонные ленты и посмотреть дифф
QCC_UPDATE_GOLDEN=1 cargo test -p provider-claude-code --test conformance
```

## Свой плагин

Плагин — компонент `wasm32-wasip2` плюс `plugin.toml`. Смотрите `plugins/provider-echo`: это
полный пример на SDK, а `crates/qcc-plugin-testkit/tests/echo.rs` показывает, как прогонять мост
через настоящий хост и сравнивать ленту с эталоном (`QCC_UPDATE_GOLDEN=1` перезаписывает эталоны).

### UI плагина

Плагин может добавить в приложение свой экран, вкладку треда и рисовать элементы ленты своего
вида. Он не присылает разметку или код, а отвечает деревом узлов из фиксированного набора
(`stack`, `row`, `heading`, `text`, `markdown`, `code`, `badge`, `keyValues`, `table`, `button`,
`input`, `divider`); приложение рисует известные узлы и пропускает остальные.

```rust
use qcc_plugin_sdk::{prelude::*, ui};   // feature "ui"

impl Ui for Notes {
    fn views(&self) -> Vec<UiViewInfo> {
        vec![UiViewInfo {
            id: "notes".into(),
            title: "Notes".into(),
            placement: UiPlacement::ThreadTab,   // или App: отдельный экран
            scope: UiScope::AllThreads,          // или OwnThreads: только треды своих провайдеров
        }]
    }

    fn render(&mut self, request: UiRender) -> Result<String, String> {
        Ok(ui::stack([ui::heading("Notes"), ui::button("Add", "add").primary()]).to_json())
    }

    // Нажатие кнопки или отправка поля ввода; ответ заменяет вид.
    fn action(&mut self, action: UiAction) -> Result<String, String> { /* … */ }
}
```

Элемент ленты своего вида (`ItemShape::Extension`, вид всегда `<id-плагина>/<имя>`) рисуется через
`item_kinds` и `render_item`; чужие виды плагин перехватить не может. Рабочий пример — вкладка
«Echo» и команда `/card` в `plugins/provider-echo`. В приложении вклады плагинов регистрируются
в том же реестре, что и встроенные модули (`apps/desktop/src/modules/plugin-ui`).

Установленный плагин лежит в `<data>/plugins/<id>/`. Стороннему плагину права выдаются файлом
`<data>/grants/<id>.toml` в формате секции `[permissions]` манифеста; без него плагин загружается
без прав.
