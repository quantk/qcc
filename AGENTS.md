# AGENTS.md

Инструкции для кодовых агентов (Claude Code, Codex, opencode), работающих с этим репозиторием.
Что такое qcc — в [README.md](README.md); как им пользоваться и как он устроен — в
[docs/](docs/README.md); устройство desktop-приложения для разработчика — в
[apps/desktop/README.md](apps/desktop/README.md).

## Коротко о проекте

qcc — центр управления кодовыми агентами на Rust. Своего агентского цикла нет: работают реальные
CLI (Claude Code, Codex, opencode), а qcc ведёт сессии (треды), хранит их ленту и расширяется
WASM-плагинами.

- `qccd` — демон: ядро, журнал SQLite, хост плагинов, JSON-RPC по unix-сокету.
- `qcc` — CLI, клиент демона.
- `apps/desktop` — Tauri + React + shadcn/ui, «пульт управления» агентами.
- Провайдеры агентов, git worktree, оркестрация — это **плагины** (`wasm32-wasip2`-компоненты по
  контракту `wit/plugin.wit`).

## Принципы, которые нельзя нарушать

- **Мост знает диалект, ядро знает ленту.** Всё, что специфично для агента (формат его протокола,
  флаги, способ доставки skill'ов), живёт в плагине-провайдере. Ядро работает только с доменными
  типами ленты.
- **Ядро никогда не ветвится по id провайдера или плагина.** Если хочется написать
  `if provider == "codex"` в `crates/` — нужна возможность в контракте, которую провайдер объявляет.
- **У встроенных плагинов нет привилегий.** Они собираются из тех же исходников, тем же SDK и в тот
  же артефакт, что и сторонние (`crates/qcc-plugins-bundle/build.rs`). Не давайте им обходных путей
  в демон.
- **Права явные.** Каждая возможность хоста (kv, процессы, `threads`, `skills`, …) — отдельное право
  в `plugin.toml`; новая возможность — новое право, проверяемое хостом.
- **UI модульный.** Новая функция desktop-приложения — это модуль в `apps/desktop/src/modules/`,
  регистрирующий свои вклады через `src/ui/registry.ts` и перечисленный в `modules/index.ts`, а не
  особый случай в `shell/`. То, что про агента полезно видеть сразу, кладите в проекцию `Thread`,
  чтобы обзор показывал это без чтения лент.

## Карта репозитория

| Путь | Что там |
| --- | --- |
| `crates/qcc-domain` | доменные типы: треды, лента, элементы, провайдеры, команды |
| `crates/qcc-runtime` | сборщик дельт в ленту, движок |
| `crates/qcc-store` | SQLite-журнал событий и проекции |
| `crates/qcc-plugin-host` | wasmtime-хост: загрузка, права, импорты (`actor.rs`, `convert.rs`) |
| `crates/qcc-plugin-sdk` | SDK для плагинов (трейты `Plugin`, `Commands`, `skills_prompt`, …) |
| `crates/qcc-plugin-testkit` | прогон плагина без демона, golden-ленты |
| `crates/qcc-plugins-bundle` | собирает `plugins/*` в wasm и встраивает в демон |
| `crates/qcc-protocol` | методы JSON-RPC, пути и переменные окружения (`paths.rs`) |
| `crates/qcc-client` | клиент протокола |
| `crates/qcc-daemon` | `qccd`: `core/` (состояние и реакции ядра: треды, сессии, окружения, ожидания), `plugins.rs`, `server.rs` |
| `crates/qcc-cli` | `qcc`; e2e-тесты в `tests/e2e.rs` |
| `plugins/provider-*` | мосты к агентам: `claude-code`, `codex`, `acp` (opencode), `echo` (тестовый) |
| `plugins/env-git-worktree` | окружение треда в отдельном git worktree |
| `plugins/orchestration` | skill `qcc-orchestration` и команды `handoff`/`review` |
| `wit/plugin.wit` | контракт плагинов |
| `apps/desktop` | фронтенд (`src/`) и Tauri-оболочка (`src-tauri/`) |
| `scripts/` | проверки; `fixtures/` — запись и редактирование фикстур реальных агентов |
| `docs/` | пользовательская документация (английский); `docs/images/` — скриншоты для README |

## Окружение и команды

Всё запускается через nix flake. `./dev` сам заходит в нужный shell, поэтому работает из любого
терминала.

```sh
./dev check                 # fmt + clippy -D warnings + все тесты (= scripts/check.sh)
./dev test [ARGS]           # cargo test --workspace --exclude qcc-desktop [ARGS]
./dev                       # собрать и запустить qccd на переднем плане (данные в ./.dev)
./dev qcc thread list       # CLI против dev-демона
./dev demo                  # несколько echo-тредов через временный демон
./dev desktop               # приложение против dev-демона
./dev app                   # release-сборка для ежедневной работы в ~/.local/lib/qcc (данные в ~/.local/share/qcc)
nix develop .#desktop -c ./scripts/check-desktop.sh   # clippy desktop, tsc, vitest, vite build
```

Если вы уже в `nix develop`, можно вызывать `cargo` напрямую. `qcc-desktop` исключён из
`default-members`: ему нужны GUI-библиотеки из `.#desktop`.

**Перед коммитом** должны проходить `./scripts/check.sh` и, если трогали `apps/desktop` или
протокол, `scripts/check-desktop.sh`.

## Как вносить типичные изменения

- **Метод протокола / тип ответа.** Правьте `crates/qcc-protocol`, затем перегенерируйте
  TypeScript: `QCC_UPDATE_TS=1 cargo test -p qcc-protocol --test typescript`. Файл
  `apps/desktop/src/api/protocol.gen.ts` коммитится; тест падает, если он устарел. Руками его
  не правьте.
- **Контракт плагинов.** Меняйте `wit/plugin.wit`, затем по цепочке: типы в `qcc-domain` →
  `qcc-plugin-host` (`api`, `convert.rs`, `actor.rs`) → `qcc-plugin-sdk` → testkit → плагины.
  Новая возможность хоста — с новым правом в `Permissions` и в `plugin.toml` тех плагинов, кому
  оно нужно.
- **Новый плагин.** Каталог `plugins/<имя>` с `Cargo.toml`, `plugin.toml` (id, версия, `api`,
  права) и `src/lib.rs` на SDK; он автоматически попадёт в bundle. Пример с UI и командами —
  `plugins/provider-echo`, с тредами и skill'ом — `plugins/orchestration`.
- **Поведение провайдера.** Тесты моста — conformance по записанным фикстурам
  (`plugins/provider-*/fixtures/*.ndjson` → `*.timeline.jsonl`). Если лента изменилась намеренно,
  перезапишите golden: `QCC_UPDATE_GOLDEN=1 cargo test -p <плагин>`. Новые записи реальных агентов
  делаются скриптами `scripts/fixtures/record-*.py` и обязательно проходят через
  `scripts/fixtures/redact.py` перед коммитом (пути, домашний каталог, идентификаторы).
- **Desktop.** Новый модуль в `src/modules/<имя>/`, регистрация в `modules/index.ts`. Логику
  состояния держите в `src/state/` с vitest-тестами рядом (`*.test.ts`).
- **UI-компоненты — shadcn/ui** (стиль `base-nova` на Base UI, Tailwind v4, иконки `lucide-react`).
  Не пишите свои кнопки, поля, диалоги, меню и select'ы и не тащите другую UI-библиотеку: берите
  компонент из `src/components/ui/` или добавьте новый (`pnpm dlx shadcn@latest add <имя>` в
  `apps/desktop`). Добавленный компонент — наш код, его можно править. Если CLI висит (бывает),
  возьмите `https://ui.shadcn.com/r/styles/base-nova/<имя>.json`, перепишите импорты
  `@/registry/base-nova/ui/` на `@/components/ui/`, а `<IconPlaceholder lucide="X" …/>` замените на
  `<X …/>`, сохранив остальные пропсы.
- **Паттерны AI Elements** (Tool, Terminal, Reasoning, Confirmation, Chat, Prompt Input, Context) —
  это композиции компонентов shadcn, а не пакет. Они собраны в `src/ui/kit` (`AgentTrace.tsx`,
  `InteractionPrompt.tsx`) и питаются элементами нашей ленты; Vercel AI SDK (`ai`, `useChat`)
  не подключаем. Новый вид элемента ленты рисуйте через `ItemTrace` из kit.
- **Цвета и стили.** Токены shadcn (`--primary`, `--muted`, …) в `src/theme.css` указывают на
  палитру приложения (`--brand`, `--text-muted`, `--panel`, … в `src/styles.css`); не заводите
  цвета мимо них. Старый CSS лежит в `@layer base`, чтобы утилиты Tailwind его перебивали; новые
  стили пишите классами Tailwind, а не правилами в `styles.css`. Подробнее — раздел «Styling» в
  `apps/desktop/README.md`.
- Dev-сервер Vite иногда отдаёт устаревший CSS после пересоздания файла компонента: перед тем как
  верить скриншоту, перезапустите `./dev desktop`.

## Тестирование

- Unit-тесты рядом с кодом; интеграционные — в `crates/*/tests` и `plugins/*/tests`; сквозные
  сценарии CLI + демон + плагины — `crates/qcc-cli/tests/e2e.rs` (на провайдере `echo`).
- Провайдер `echo` — тестовый агент: `/slow`, `/tool`, `/ask`, `/approve`, `/exec`, `/fail`,
  `/skills`, `/mcp`, `/agent`, `/crash` и др. (см. doc-комментарий `plugins/provider-echo/src/lib.rs`).
  Новые сценарии ядра проверяйте на нём, а не на реальных агентах.
- **Живые проверки на реальных агентах** стоят денег и меняют файлы. Если они нужны:
  - запускайте отдельный демон с `QCC_DATA_DIR`/`QCC_SOCKET` во временном каталоге, а рабочий
    каталог треда — тоже во временном (не `~` и не этот репозиторий);
  - для Claude используйте `--model haiku`;
  - Codex на этой машине может не спрашивать разрешений (авто-ревьюер в его конфиге), а opencode
    по умолчанию разрешает всё — это не обязательно баг моста;
  - внутри сессий Codex `PATH` пересобирается login-шеллом, поэтому агентам передаётся `QCC_BIN`.
- **GUI** проверяйте на виртуальном дисплее, а не на экране пользователя: `Xvfb :99`,
  `DISPLAY=:99 GDK_BACKEND=x11 LIBGL_ALWAYS_SOFTWARE=1 WEBKIT_DISABLE_COMPOSITING_MODE=1`,
  `WAYLAND_DISPLAY` снят, свой `QCC_DATA_DIR`. Скриншоты и ввод — `imagemagick`/`xdotool` через
  `nix shell` с ревизией nixpkgs из `flake.lock`.

## Осторожно

- **Не убивайте чужие процессы.** У пользователя может работать свой `./dev desktop` / `qccd` с
  данными в `./.dev`. Не останавливайте его и не трогайте `./.dev` (`./dev clean` удаляет его).
  Его рабочее приложение из `./dev app` живёт в `~/.local/lib/qcc` с данными в
  `~/.local/share/qcc`: не запускайте `./dev app` без `QCC_APP_DIR`/`XDG_DATA_HOME`/
  `XDG_RUNTIME_DIR` во временном каталоге — команда переоткрывает его окно и перезапускает демон.
- **Никогда `pkill -f`**: шаблон совпадает с командой самого шелла и убивает его. Используйте
  `pkill -x qccd`, `pkill -x qcc-desktop` или PID.
- **Вы сами можете быть внутри треда qcc.** Тогда в окружении есть `QCC_THREAD_ID`, `QCC_SOCKET`,
  `QCC_BIN`, и `qcc thread spawn` создаст потомка вашего треда. Для тестов снимайте
  `QCC_THREAD_ID` и задавайте свой `QCC_SOCKET` (так делает `./dev`).
- Глубина делегирования ограничена (`MAX_SPAWN_DEPTH` в `crates/qcc-daemon/src/core/mod.rs`).

## Стиль

- Rust edition 2024, `unsafe_code = "forbid"`, clippy с `-D warnings`, формат — `rustfmt.toml`.
  Не оставляйте `dbg!` и `todo!`.
- Комментарии и doc-комментарии в коде — на английском, полными предложениями, объясняют «почему».
  Документация — на английском: `README.md`, `docs/`, `apps/desktop/README.md`. `README.ru.md` —
  русская версия витрины `README.md`; меняя одну, меняйте и другую.
- Сообщения коммитов — на английском, в повелительном наклонении или как короткое описание
  результата (`Stage 7: orchestration through the CLI, threads import and commands`). Один логический
  шаг — один коммит.
- **Работаем прямо в `main`.** Веток, worktree и PR не заводите, если об этом явно не попросили;
  коммиты — в `main`.
- **Коммитьте по ходу работы, не спрашивая.** Закончили осмысленный кусок и следующий шаг уже про
  другое — прогоните проверки, закоммитьте и идите дальше. Не копите всё в один коммит в конце.
- Меняя поведение, видимое пользователю (команды CLI, права, возможности провайдеров), обновляйте
  нужный файл в `docs/` (для приложения — `docs/desktop.md`, для разработчиков приложения —
  `apps/desktop/README.md`); `README.md` и `README.ru.md` — если меняется то, что показано на
  витрине.
