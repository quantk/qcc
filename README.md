# qcc

Центр управления кодовыми агентами. Своего агентского цикла нет: работают Claude Code, Codex и
opencode, а qcc ведёт параллельные сессии, хранит их ленту и расширяется плагинами.
Архитектура и этапы — в [PLAN.md](PLAN.md).

## Состояние

Готовы этапы 0–3: домен и сборщик дельт, SQLite-журнал, хост WASM-плагинов с правами, SDK,
демон `qccd` и CLI `qcc`. Единственный провайдер пока `echo` — тестовый и одновременно пример
плагина. Реальные агенты, worktree, desktop и оркестрация — этапы 4–7.

## Быстрый старт

Всё окружение описано во `flake.nix` (rust с таргетом `wasm32-wasip2`, `wasm-tools`):

```sh
nix develop
cargo build

./target/debug/qccd &                                   # демон
./target/debug/qcc plugin list
./target/debug/qcc thread spawn --provider echo --prompt 'hello' --follow
./target/debug/qcc thread list
./target/debug/qcc thread show <thread-id>
```

У каждой команды есть `--json`. Данные лежат в `~/.local/share/qcc` (`QCC_DATA_DIR`), сокет —
в `$XDG_RUNTIME_DIR/qcc/qccd.sock` (`QCC_SOCKET`).

Проверки, которые должен проходить каждый этап:

```sh
./scripts/check.sh      # fmt, clippy -D warnings, тесты
```

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
