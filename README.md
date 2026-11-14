# qcc

Центр управления кодовыми агентами. Своего агентского цикла нет: работают Claude Code, Codex и
opencode, а qcc ведёт параллельные сессии, хранит их ленту и расширяется плагинами.
Архитектура и этапы — в [PLAN.md](PLAN.md).

## Состояние

Готовы этапы 0–3: домен и сборщик дельт, SQLite-журнал, хост WASM-плагинов с правами, SDK,
демон `qccd` и CLI `qcc`. Единственный провайдер пока `echo` — тестовый и одновременно пример
плагина. Реальные агенты, worktree, desktop и оркестрация — этапы 4–7.

## Быстрый старт

Одна команда, из любого шелла (сама заходит в `nix develop`):

```sh
./dev demo      # поднять демон, прогнать несколько echo-тредов, остановить
./dev           # qccd на переднем плане, данные в ./.dev
./dev qcc thread spawn --provider echo --prompt 'hello' --follow   # CLI к этому демону
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
```

Главное правило: **мост знает диалект, ядро знает ленту.** Плагин-провайдер переводит вывод
агента в узкий набор дельт; id, порядок и жизненный цикл ходов и элементов принадлежат сборщику
в `qcc-runtime`. Ядро нигде не ветвится по id провайдера, а встроенные плагины идут тем же путём,
что и сторонние.

## Свой плагин

Плагин — компонент `wasm32-wasip2` плюс `plugin.toml`. Смотрите `plugins/provider-echo`: это
полный пример на SDK, а `crates/qcc-plugin-testkit/tests/echo.rs` показывает, как прогонять мост
через настоящий хост и сравнивать ленту с эталоном (`QCC_UPDATE_GOLDEN=1` перезаписывает эталоны).

Установленный плагин лежит в `<data>/plugins/<id>/`. Стороннему плагину права выдаются файлом
`<data>/grants/<id>.toml` в формате секции `[permissions]` манифеста; без него плагин загружается
без прав.
