# qcc

Центр управления кодовыми агентами. Своего агентского цикла нет: работают Claude Code, Codex и
opencode, а qcc ведёт параллельные сессии, хранит их ленту и расширяется плагинами.
Архитектура и этапы — в [PLAN.md](PLAN.md).

## Состояние

Готовы этапы 0–4: домен и сборщик дельт, SQLite-журнал, хост WASM-плагинов с правами, SDK,
демон `qccd`, CLI `qcc` и провайдеры реальных агентов. Worktree, desktop и оркестрация —
этапы 5–7.

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

## Быстрый старт

Одна команда, из любого шелла (сама заходит в `nix develop`):

```sh
./dev demo      # поднять демон, прогнать несколько echo-тредов, остановить
./dev           # qccd на переднем плане, данные в ./.dev
./dev qcc thread spawn --provider echo --prompt 'hello' --follow   # CLI к этому демону
./dev qcc thread spawn --provider claude-code --permission-mode ask --prompt 'что в этом каталоге?' --follow
./dev qcc thread send <thread-id> 'и ещё вот что' --steer
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
plugins/provider-echo     пример провайдера
plugins/provider-claude-code, provider-acp, provider-codex   мосты к реальным агентам
```

Главное правило: **мост знает диалект, ядро знает ленту.** Плагин-провайдер переводит вывод
агента в узкий набор дельт; id, порядок и жизненный цикл ходов и элементов принадлежат сборщику
в `qcc-runtime`. Ядро нигде не ветвится по id провайдера, а встроенные плагины идут тем же путём,
что и сторонние.

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

Установленный плагин лежит в `<data>/plugins/<id>/`. Стороннему плагину права выдаются файлом
`<data>/grants/<id>.toml` в формате секции `[permissions]` манифеста; без него плагин загружается
без прав.
