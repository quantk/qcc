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

Агенты работают со своими настройками: модель по умолчанию, права и песочница берутся из
конфигурации самого агента, qcc их не ужесточает и не ослабляет. Запросы разрешений, которые агент
адресует пользователю, приходят в ленту и закрываются через `qcc interaction resolve`.

## Быстрый старт

Одна команда, из любого шелла (сама заходит в `nix develop`):

```sh
./dev demo      # поднять демон, прогнать несколько echo-тредов, остановить
./dev           # qccd на переднем плане, данные в ./.dev
./dev qcc thread spawn --provider echo --prompt 'hello' --follow   # CLI к этому демону
./dev qcc thread spawn --provider claude-code --prompt 'что в этом каталоге?' --follow
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
