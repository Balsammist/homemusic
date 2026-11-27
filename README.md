# HomeMusic

HomeMusic — домашняя музыкальная библиотека с веб-интерфейсом и отдельным GTK-плеером. Navidrome хранит библиотеку и учётные записи. Hub отвечает за поиск, воспроизведение, загрузки, импорт и рекомендации. Плеер и веб-интерфейс работают с одним API.

| Каталог | Что в нём |
| --- | --- |
| `native/` | Desktop-клиент на Rust, GTK4 и GStreamer, сейчас для Linux |
| `hub/` | FastAPI, SQLite, миграции Alembic, интеграция с Navidrome и источниками музыки |
| `web/` | React, TypeScript и Vite, устанавливаемый веб-интерфейс (PWA) |
| `embedder/` | Опциональный сервис аудиовекторов для рекомендаций |
| `navidrome/` | Конфигурация Navidrome |
| `gateway/` | Опциональный SSH-туннель и настройка Caddy на VPS |
| `eval/`, `scripts/` | Оценка ассистента и скрипт выпуска отдельного Android-клиента |

## Сервер и веб на Linux или Windows

Нужны Docker и Compose v2. На Windows используйте Docker Desktop с WSL2 и Linux-контейнерами. Из корня репозитория на Linux, macOS или в Git Bash на Windows запустите следующие команды.

```sh
cp .env.example .env
cp config.example.yaml config.yaml
mkdir -p music/library music/staging music/.trash music/.backups
```

В PowerShell для копирования используйте `Copy-Item .env.example .env` и `Copy-Item config.example.yaml config.yaml`. Папки для музыки можно создать через Проводник. По умолчанию `MUSIC_ROOT=./music`. Для отдельного диска укажите абсолютный путь, например `D:/Music`, и создайте те же подпапки в нём.

Заполните в `.env`:

- `NAVIDROME_ADMIN_USER` и `NAVIDROME_ADMIN_PASSWORD` должны совпадать с логином и паролем первого администратора в Navidrome. Для первого запуска создайте его в веб-интерфейсе Navidrome.
- `ND_PASSWORDENCRYPTIONKEY` — своя случайная строка. Пример генерации с Python 3: `python -c "import secrets; print(secrets.token_hex(32))"`.
- `HUB_SECRET_KEY` — отдельный ключ Fernet. Команда без дополнительных библиотек: `python -c "import secrets,base64; print(base64.urlsafe_b64encode(secrets.token_bytes(32)).decode())"`.

Сохраните оба ключа для резервного копирования. Приложения используют их для шифрования сохранённых учётных данных. Для примера выше пароль в `.env` лучше задавать без пробелов, кавычек, `$` и `#`, чтобы Compose и desktop-клиент прочитали одно значение.

```sh
docker compose config --quiet
docker compose up -d --build
docker compose ps
```

1. Откройте Navidrome на `http://localhost:4533`, создайте администратора и при необходимости запустите сканирование `music/library`.
2. Откройте HomeMusic на `http://localhost:8080` и войдите с теми же учётными данными.
3. Проверка сервера доступна на `http://localhost:8080/healthz`, Swagger — на `http://localhost:8080/api/docs`.

Первый запуск скачивает зависимости и образы. Веб собирается при сборке Hub и обслуживается из того же контейнера. `HomeMusic.bat` запускает и останавливает серверный стек после настройки на Windows.

По умолчанию порты доступны только на этой машине. Для доступа с другой машины используйте HTTPS через обратный прокси и настройте `PUBLIC_HUB_URL` и `PUBLIC_MUSIC_URL` под фактические адреса. `HOMEMUSIC_BIND_ADDRESS` управляет привязкой портов. Для обновления повторите `docker compose up -d --build`. Для остановки выполните `docker compose down`. При резервном копировании сохраняйте музыкальную папку, тома SQLite, `.env` и `config.yaml`. `docker compose down -v` удаляет тома с базами.

### Дополнительные службы

- YouTube использует yt-dlp, Deno и внутренний сервис `youtube-pot`. Доступность и полнота аудио зависят от внешних источников и сети сервера.
- Для аудиовекторов нужны Linux-контейнеры с доступом к NVIDIA GPU, совместимым с CUDA 12.8, и место для модели и зависимостей. Задайте `EMBEDDER_URL=http://embedder:8090` и запустите `docker compose --profile gpu up -d --build`. Без профиля работают остальные источники рекомендаций.
- Для SSH-шлюза настройте свой VPS, `VPS_HOST`, `gateway/tunnel/tunnel_key` и `gateway/tunnel/known_hosts`, затем включите профиль `gateway`. [Схема и настройка шлюза](docs/upstream.md)
- Last.fm, ассистент, SMTP и вход Google включаются своими параметрами в [примере окружения](.env.example). Развёртывание локальной библиотеки этих ключей не требует.

## Веб-разработка

Нужен Node.js 22.12 или новее и npm. Установите зависимости по lock-файлу:

```sh
cd web
npm ci
npm run dev
```

Vite слушает `http://localhost:5173` и проксирует API к локальному Hub на порту 8080. `npm run dev:mock` поднимает интерфейс с тестовыми ответами без Hub. Учётные данные для mock-режима — `test` / `test`. `npm run build` проверяет TypeScript и пишет сборку в `hub/app/static/`. Точки входа — `web/src/` и `web/vite.config.ts`.

## Разработка Hub

Нужны Python 3.12, uv и FFmpeg/ffprobe в `PATH`. Для работы с YouTube вне Compose отдельно установите Deno и укажите URL bgutil через `YTDLP_POT_BASE_URL`. Для тестов отдельный экземпляр Navidrome не нужен.

```sh
cd hub
uv sync --locked
uv run --no-sync pytest -q
```

Чтобы отлаживать Hub без его контейнера, остановите контейнер Hub и оставьте Navidrome на порту 4533. Из `hub/` в bash:

```sh
MUSIC_ROOT="$PWD/../music" DATA_DIR="$PWD/data" CONFIG_FILE="$PWD/../config.yaml" \
NAVIDROME_URL=http://127.0.0.1:4533 \
uv run --no-sync uvicorn app.main:app --env-file ../.env --host 127.0.0.1 --port 8080 --reload
```

Это отдельная локальная база Hub в `hub/data/`. Для тестового запуска без фоновых задач задайте `DISABLE_BACKGROUND=true`. Веб-сборку следует создать до старта Hub, чтобы он подключил статические файлы.

Основные места для изменений — `hub/app/api/` (HTTP), `hub/app/providers/` (источники), `hub/app/listen.py` (воспроизведение), `hub/app/mixes/` (рекомендации), `hub/migrations/` (схема БД). [Описание API](hub/API.md)

Для тестов embedder на Linux из `embedder/` выполните `uv sync --locked`, затем `uv run --no-sync pytest -q`. Обычный набор использует тестовую модель. Дополнительные проверки реальной модели требуют соответствующих зависимостей и весов.

## Desktop-клиент на Linux

Для сборки нужны Rust 1.92 или новее (проверено на Rust 1.100), C-линкер и `pkg-config`, development-пакеты GTK 4.16+ и GStreamer 1.x. Для запуска нужны плагины GStreamer для `playbin3`, HTTP(S), декодирования аудио и системного аудиовыхода. Обычно это наборы `base`, `good` и `libav`. `uconv` из ICU используется для нормализации текста.

```sh
cargo build --release --locked --manifest-path native/Cargo.toml
./native/run.sh
```

Клиент берёт адрес Hub из `HOMEMUSIC_API_URL` (по умолчанию `http://127.0.0.1:8080`), логин и пароль — из `.env` в каталоге приложения. Лаунчер выбирает корень клона автоматически. Можно задать свой `HOMEMUSIC_ROOT`, создав внутри подпапку `native/` для настроек. Для запуска на HTTPS-сервере:

```sh
HOMEMUSIC_API_URL=https://music.example.org ./native/run.sh
```

`./native/run.sh --background` запускает клиент в фоне. Трей поддерживается окружениями с StatusNotifierItem. Linux-медиаклавиши и внешняя очередь используют D-Bus. Quickshell необязателен, а `HOMEMUSIC_THEME` позволяет указать собственный JSON с палитрой. [Импорт Spotify](native/SPOTIFY.md)

Для ярлыка в меню установите `native/homemusic-native.desktop` в пользовательский каталог приложений, создайте ссылку на `native/run.sh` в `PATH` с именем `homemusic` и установите `native/assets/homemusic.svg` как значок `homemusic`. Обычный запуск из клона обходится без установки ярлыка.

```sh
cargo test --locked --manifest-path native/Cargo.toml
cd hub
PYTHONPATH=. uv run --no-sync pytest -q ../native/tests
```

### Что посмотреть для порта на Windows

`native/src/config.rs` отвечает за пути, настройки и тестовое окружение, `api.rs` — за вход и HTTP, `audio.rs` — за GStreamer, `ui.rs` и `ui/` — за GTK. Для Windows понадобится заменить Unix-вызовы файловой системы и тестового сокета, реализовать системный трей и медиаинтеграцию вместо Linux D-Bus и собрать дистрибутив GTK/GStreamer с плагинами. Режим работы Windows-сервера описан выше. Сборка самого GTK-клиента под Windows пока не подтверждена.

## Происхождение

Основой служит [justman-1/HomeMusic-public](https://github.com/justman-1/HomeMusic-public), коммит `59c05b4`. Включены текущие локальные доработки Hub и самостоятельный GTK-клиент. [Документация upstream](docs/upstream.md) содержит дополнительные сведения о функциях. Для запуска этой копии используйте команды из текущего README. Исходники Android-клиента хранятся отдельно.
