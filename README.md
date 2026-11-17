# Minecraft Mods API

REST API файлового каталогу модів на **Rust + Axum + PostgreSQL**. Аутентифікація —
JWT, доступ — динамічна рольова система: правила (permissions) є рядками в БД, їх
можна створювати в рантаймі й видавати ролям або конкретному користувачу.

Каталог модів — це **вміст папки `MODS_DIR`**, а не таблиця в БД: список, завантаження,
вивантаження й видалення працюють з файлами, а кожна дія з файлом пишеться в журнал
`mod_file_actions` (хто, що, коли).

| Шар | Вибір |
|---|---|
| Runtime | Rust 2024, Tokio |
| HTTP | Axum 0.8 |
| БД | PostgreSQL + SQLx (міграції вбудовані в бінарник) |
| Auth | JWT (HS256), Argon2id для паролів |
| Docs | utoipa + Swagger UI на `/docs` |
| Конфіг | `.env` (dotenvy), валідація на старті |

## Швидкий старт

```bash
cp .env.example .env      # заповнити DATABASE_URL, JWT_SECRET, ADMIN_*
docker compose up -d db   # PostgreSQL 18 на localhost:5432
cargo run
```

Міграції застосовуються автоматично при старті (`sqlx::migrate!`), окремий
`sqlx-cli` для запуску не потрібен.

Адмін-панель (Angular 22 + Material) лежить у [`../admin-panel`](../admin-panel):
`npm install && npm start` → <http://localhost:4200>, dev-проксі на цей бекенд уже
налаштований. Вхід — адміністратором або акаунтом із роллю `user`: такий акаунт
бачить лише розділ **Mods** і може створювати користувачів із власним набором
прав.

Стан проєкту, ухвалені рішення, пастки середовища й план доробки — у
[`HANDOFF.md`](HANDOFF.md).

- API: <http://localhost:8080/api/v1>
- Swagger UI: <http://localhost:8080/docs>
- OpenAPI JSON: <http://localhost:8080/api-docs/openapi.json>
- Health: <http://localhost:8080/health>

## Конфігурація

| Змінна | Обов'язкова | Типово | Призначення |
|---|---|---|---|
| `DATABASE_URL` | так | — | `postgres://user:pass@host:5432/db` |
| `JWT_SECRET` | так | — | мінімум 32 символи; ротація відкликає всі токени |
| `JWT_TTL_MINUTES` | ні | `60` | час життя access-токена |
| `HOST` / `PORT` | ні | `0.0.0.0` / `8080` | адреса прослуховування |
| `CORS_ORIGINS` | ні | `*` | список через кому або `*` |
| `MODS_DIR` | ні | `./data/mods` | папка з файлами модів; створюється на старті, шлях канонікалізується |
| `MODS_MAX_UPLOAD_BYTES` | ні | `134217728` | максимальний розмір одного файлу (128 MiB) |
| `ADMIN_EMAIL`, `ADMIN_USERNAME`, `ADMIN_PASSWORD` | разом | — | перший адміністратор; задаються всі три або жодна |
| `PORTAINER_URL`, `PORTAINER_API_KEY` | разом | — | Portainer для керування контейнером Minecraft; без них `/server/*` → `503` |
| `PORTAINER_ENDPOINT_ID` | ні | `1` | environment (endpoint) Portainer |
| `MC_CONTAINER` | ні | `minecraft` | ім'я контейнера Minecraft |
| `PORTAINER_VERIFY_TLS` | ні | `true` | `false` лише для самопідписаного сертифіката |
| `PORTAINER_TIMEOUT_SECS` | ні | `15` | таймаут запиту до Portainer |
| `RUST_LOG` | ні | `info,tower_http=debug` | фільтр логів |

## Авторизація

Публічної реєстрації немає. Перший адміністратор з'являється з `ADMIN_EMAIL` /
`ADMIN_USERNAME` / `ADMIN_PASSWORD` при першому старті на порожній БД; після
створення ці змінні можна прибрати.

- **Адміністратор** (`admin`) керує користувачами, ролями й правилами та обирає
  ролі для нових акаунтів.
- **Акаунт із правом `users:create`** (типово роль `user`) може створювати
  користувачів без вибору ролей: новий акаунт отримує **точно ті самі** ролі й
  прямі видачі, що й творець. Підвищити доступ так неможливо.

```bash
# 1. Логін адміністратора
TOKEN=$(curl -s -X POST http://localhost:8080/api/v1/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"email":"admin@example.com","password":"<ADMIN_PASSWORD>"}' \
  | sed -E 's/.*"access_token":"([^"]+)".*/\1/')

# 2. Адмін створює користувача (без roles — буде роль user)
curl -X POST http://localhost:8080/api/v1/users \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"email":"steve@example.com","username":"steve","password":"correct-horse"}'

# 3. Логін створеного користувача
USER_TOKEN=$(curl -s -X POST http://localhost:8080/api/v1/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"email":"steve@example.com","password":"correct-horse"}' \
  | sed -E 's/.*"access_token":"([^"]+)".*/\1/')

# 4. Поточний користувач з ефективними правами
curl http://localhost:8080/api/v1/auth/me -H "Authorization: Bearer $USER_TOKEN"

# 5. Зміна власного пароля (поточний перевіряється)
curl -X PUT http://localhost:8080/api/v1/auth/password \
  -H "Authorization: Bearer $USER_TOKEN" -H 'Content-Type: application/json' \
  -d '{"current_password":"correct-horse","new_password":"correct-horse-2"}'
```

Пароль зберігається як Argon2id-хеш. Хешування виконується в
`spawn_blocking`, щоб не блокувати async-рантайм. `AuthUser` перечитує
користувача з БД на кожен запит, тож `is_active = false` діє негайно.

Кожен автентифікований акаунт може змінити **власний** пароль через
`PUT /auth/password`: поточний пароль перевіряється, новий має бути 8–128
символів і відрізнятися від поточного. Хибний поточний пароль дає `400`
(не `401`), щоб клієнт не сприйняв це як протухлий токен. Видані раніше JWT
лишаються дійсними до `exp` — окремого `token_version` немає, тому після зміни
пароля на інших пристроях треба вийти вручну. Пароль **іншого** користувача
змінює лише адміністратор через `PATCH /users/{id}`.

## Динамічна рольова система

Ефективні права користувача = **права його ролей ∪ прямі видачі користувачу**.
Усе перевіряється запитом до БД, тому зміни діють з наступного запиту.

```
users ──< user_roles >── roles ──< role_permissions >── permissions
  └────< user_permissions >──────────────────────────────┘
```

Створити нове правило й видати його — це два запити, без деплою. Усі кроки
нижче виконує **адміністратор** (роль `admin`), бо вони змінюють доступ інших
людей:

```bash
# 1. Нове правило
curl -X POST http://localhost:8080/api/v1/permissions \
  -H "Authorization: Bearer $ADMIN_TOKEN" -H 'Content-Type: application/json' \
  -d '{"code":"mods:publish","description":"Publish mods to the catalog"}'

# 2. Роль з набором правил (неіснуючий код відхилить весь запит)
curl -X POST http://localhost:8080/api/v1/roles \
  -H "Authorization: Bearer $ADMIN_TOKEN" -H 'Content-Type: application/json' \
  -d '{"name":"moderator","permissions":["mods:read","mods:download","mods:upload"]}'

# 3. Видати роль користувачу
curl -X POST http://localhost:8080/api/v1/users/$USER_ID/roles/$ROLE_ID \
  -H "Authorization: Bearer $ADMIN_TOKEN"

# 4. Або видати правило напряму, повз роль
curl -X POST http://localhost:8080/api/v1/users/$USER_ID/permissions/$PERMISSION_ID \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

При старті сідер гарантує наявність каталогу правил:

```
mods:read, mods:download, mods:upload, mods:delete,
users:read, users:create,
roles:read,
permissions:read,
audit:read,
server:manage
```

Роль `admin` (system) на кожному старті повертає собі весь каталог — щоб
випадкове видалення правила не заблокувало адміністраторів назавжди. Роль
`user` (system) при створенні отримує базовий набір оператора Mods:
`mods:read/download/upload/delete`, `server:manage`, `users:create` і
`audit:read`. Якщо адміністратор забере ці права, перезапуск їх не поверне.

Перегляд **історії конкретного файлу** (`/mods/{file_name}/history`) доступний за
`mods:read`, бо це частина розділу Mods. Загальний журнал (`/mod-actions`)
вимагає `audit:read`: там усі дії й персональні дані, але оператори Mods бачать
його без прав на керування доступами.

System-ролі (`admin`, `user`) не можна видалити чи перейменувати, але їхній
набір правил редагується. Видалення правила каскадно прибирає його з ролей і
прямих видач.

### Чому керування — це перевірка ролі, а не permission

Редагування/видалення користувачів, їхніх прав, CRUD ролей і правил доступні
**тільки ролі `admin`** (`rbac::require_admin`). Це навмисно перевірка імені ролі,
а не динамічного правила: якби керування вмикалося правом `permissions:write`,
будь-яка роль із цим правом могла б створити собі нове правило або видати собі
`admin` — тобто підвищити власний доступ.

Єдиний делегований виняток — **створення** користувача за правилом
`users:create`. Воно безпечне, бо творець не обирає ролі: бекенд копіює його
власні `user_roles` і `user_permissions` у новий акаунт у тій самій транзакції.
Новий користувач ніколи не отримує більше за творця, а спроба передати `roles`
від не-адміна відхиляється `400`.

Динамічні permission лишаються для прикладного доступу (моди, сервер, читання
довідників), де таке самопідвищення неможливе.

Правило `admin` не можна «втратити» через API: роль системна, а її набір прав
відновлюється з каталогу на кожному старті.

## Моди: файли на диску

Джерело істини — папка `MODS_DIR` (типово `./data/mods`, створюється на старті).
База зберігає лише журнал дій; метаданих модів (опис, версія) немає, бо вони
беруться з самого файлу.

| Дія | Правило |
|---|---|
| перелік файлів | `mods:read` |
| завантаження | `mods:download` |
| вивантаження | `mods:upload` |
| видалення | `mods:delete` |
| перегляд історії файлу | `mods:read` |
| завантаження всіх файлів архівом | `mods:download` |
| загальний журнал | `audit:read` |

```bash
# Перелік (ім'я, розмір, mtime), ?limit=&offset=
curl -s http://localhost:8080/api/v1/mods -H "Authorization: Bearer $TOKEN"

# Вивантаження: multipart, поле `file` (те саме робить Swagger UI)
curl -X POST http://localhost:8080/api/v1/mods \
  -H "Authorization: Bearer $TOKEN" \
  -F "file=@sodium-0.6.13.jar"

# Завантаження: octet-stream + Content-Disposition: attachment
curl -OJ http://localhost:8080/api/v1/mods/sodium-0.6.13.jar/download \
  -H "Authorization: Bearer $TOKEN"

# Усі файли одним ZIP (будується на льоту)
curl -OJ http://localhost:8080/api/v1/mods/archive \
  -H "Authorization: Bearer $TOKEN"

# Видалення
curl -X DELETE http://localhost:8080/api/v1/mods/sodium-0.6.13.jar \
  -H "Authorization: Bearer $TOKEN"
```

Особливості, які варто знати:

- **Ім'я файлу — один сегмент шляху.** `../`, `/`, `\`, керуючі символи та імена,
  що починаються з `.`, відхиляються з `400`; приховані файли не показуються в
  переліку. Це захищає від читання `.env`, якщо `MODS_DIR` випадково вказує на
  корінь проєкту.
- **Symlink не обслуговується:** файл перевіряється через `symlink_metadata`, і
  посилання, яке може вказувати за межі теки, відхиляється.
- **Вивантаження атомарне:** тіло пишеться в прихований `.upload-<uuid>` у тій
  самій теці, потім перейменовується. Обірваний аплоад не лишає «битого» моду.
  Існуюче ім'я → `409` (спершу видаліть файл, якщо треба замінити).
- **Архів усіх модів** (`GET /mods/archive`, право `mods:download`) збирається у
  прихований `.archive-<uuid>.zip` у `MODS_DIR` і віддається потоком; дескриптор
  відкривається, файл одразу видаляється з диска (на Linux дані живуть, доки
  відкритий дескриптор), тож обірване завантаження не лишає сміття. JAR вже
  стиснений, тому записи кладуться без повторного deflate. Архів **не пише**
  рядок у журнал на кожен файл — інакше один клік додавав би сотні записів.
- **Ліміт розміру** — `MODS_MAX_UPLOAD_BYTES`. Перевищення дає `413`, частково
  записаний файл видаляється. Якщо запит іде через nginx адмін-панелі, той має
  власний `client_max_body_size` (`CLIENT_MAX_BODY_SIZE`, типово `128m`): він
  мусить бути не меншим, інакше 413 поверне проксі, не дійшовши до бекенду.
- **Віддача файлу** завжди `application/octet-stream` з `Content-Disposition:
  attachment` і `X-Content-Type-Options: nosniff`, тож завантажений `.html` не
  виконається в браузері.

### Журнал дій

Кожен upload, download окремого файлу і delete пише рядок у `mod_file_actions`
(ZIP-архів усіх файлів навмисно не пише N рядків — див. вище):

| Колонка | Зміст |
|---|---|
| `id` | `bigserial` |
| `user_id` | хто виконав; `NULL`, якщо акаунт пізніше видалили |
| `action` | `upload` / `download` / `delete` (перевіряється `CHECK`) |
| `file_name` | ім'я файлу |
| `created_at` | `timestamptz`, час дії |

Ім'я та email користувача в таблиці **не дублюються**: API бере їх через
`LEFT JOIN users` у момент читання, тож журнал завжди показує актуальні дані
акаунта і не розходиться з `users`.

Уся стрічка (найновіші першими):

```bash
curl -s "http://localhost:8080/api/v1/mod-actions?limit=50" \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

Історія одного файлу — хто і що з ним робив:

```bash
curl -s "http://localhost:8080/api/v1/mods/sodium-0.6.13.jar/history?limit=50" \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

```json
[
  {
    "id": 12,
    "user_id": "ab02daef-d2dd-42b5-af98-b06b6b204c34",
    "user_email": "admin@example.com",
    "username": "admin",
    "action": "download",
    "file_name": "sodium-0.6.13.jar",
    "created_at": "2026-11-17T12:48:53.972244Z"
  }
]
```

Історія конкретного файлу доступна за `mods:read`, а загальний журнал
(`/mod-actions`) — за `audit:read`. Історія зберігається в БД і не залежить від
диска, тому:

- після `DELETE /mods/{file_name}` вона лишається доступною;
- після видалення користувача `user_id` стає `null` (`ON DELETE SET NULL`), і
  разом з ним зникає можливість показати `username`/`user_email` — ідентичності
  для цього рядка вже немає ніде. Якщо така історія потрібна назавжди, видаляйте
  користувачів деактивацією (`PATCH /users/{id}` з `is_active: false`), а не
  `DELETE`;
- для невідомого імені повертається `200 []`, а не `404` — інакше не можна було б
  відрізнити «файл ніколи не існував» від «файл видалили».

Запис робиться до початку віддачі файлу, тому навіть обірване завантаження лишає
слід. Для upload порядок зворотний: файл з'являється, і якщо рядок журналу
записати не вдалося, файл видаляється — щоб не було «тихих» змін.

## Керування сервером Minecraft (Portainer)

Моди застосовуються лише після перезапуску сервера, тому API вміє керувати
контейнером Minecraft через HTTP API Portainer. Docker-сокет у контейнер API не
монтується: Portainer уже має доступ до Docker і виступає проксі.

Налаштування (див. `.env.example`): `PORTAINER_URL`, `PORTAINER_API_KEY`
(My account → Access tokens), `PORTAINER_ENDPOINT_ID` (типово `1`),
`MC_CONTAINER` (типово `minecraft`). Якщо URL і токен не задані, `/server/*`
відповідають `503`, а решта API працює як завжди.

| Метод | Шлях | Що робить |
|---|---|---|
| `GET` | `/api/v1/server/status` | стан контейнера, образ, health, кількість рестартів |
| `POST` | `/api/v1/server/start` | запуск контейнера |
| `POST` | `/api/v1/server/stop?timeout=30` | graceful stop (`SIGTERM`, далі `SIGKILL` після таймауту) |
| `POST` | `/api/v1/server/restart?timeout=30` | те саме, але з наступним запуском |
| `GET` | `/api/v1/server/logs?tail=200&timestamps=false` | хвіст логів stdout+stderr |

```bash
curl -s http://localhost:8080/api/v1/server/status -H "Authorization: Bearer $TOKEN"
curl -s -X POST "http://localhost:8080/api/v1/server/restart?timeout=60" \
  -H "Authorization: Bearer $TOKEN"
curl -s "http://localhost:8080/api/v1/server/logs?tail=100" \
  -H "Authorization: Bearer $TOKEN"
```

Доступ — динамічне правило `server:manage` (роль `admin` отримує його при старті).
Start/stop/restart серіалізуються в процесі, щоб два одночасні натискання не
перетиналися. Логи Docker віддає мультиплексованим потоком (`stdout`/`stderr`
кадрами) — API його декодує, а для контейнерів із TTY повертає текст як є.

Типовий сценарій застосування модів: `POST /server/stop` → завантажити/видалити
файли через `/mods` → `POST /server/start`. Поки контейнер запущений, `POST /mods`
і `DELETE /mods/{file_name}` відповідають `409`: лоадер читає джарки лише при
старті, і краще не змінювати файли під працюючим сервером. Якщо Portainer не
налаштований, перевірити стан неможливо, тому обмеження не діє.

## Ендпоінти

| Метод | Шлях | Доступ |
|---|---|---|
| `POST` | `/auth/login` | публічно |
| `GET` | `/auth/me` | будь-який автентифікований |
| `PUT` | `/auth/password` | будь-який автентифікований (свій пароль) |
| `GET` | `/mods` | `mods:read` |
| `POST` | `/mods` | `mods:upload` |
| `GET` | `/mods/{file_name}/download` | `mods:download` |
| `GET` | `/mods/archive` | `mods:download` (ZIP усіх файлів, на льоту) |
| `DELETE` | `/mods/{file_name}` | `mods:delete` |
| `GET` | `/mods/{file_name}/history` | `mods:read` |
| `GET` | `/mod-actions` | `audit:read` |
| `POST` | `/users` | `users:create` (не-адмін копіює власний доступ) |
| `GET` | `/users`, `/users/{id}` | `users:read` |
| `PATCH`/`DELETE` | `/users/{id}` | **admin** |
| `POST`/`DELETE` | `/users/{id}/roles/{role_id}` | **admin** |
| `POST`/`DELETE` | `/users/{id}/permissions/{permission_id}` | **admin** |
| `GET` | `/permissions` | `permissions:read` |
| `POST`/`DELETE` | `/permissions`, `/permissions/{id}` | **admin** |
| `GET` | `/roles`, `/roles/{id}` | `roles:read` |
| `POST`/`PATCH`/`DELETE` | `/roles`, `/roles/{id}` | **admin** |
| `PUT` | `/roles/{id}/permissions` | **admin** |
| `GET` | `/server/status` | `server:manage` |
| `POST` | `/server/start` | `server:manage` |
| `POST` | `/server/stop` | `server:manage` |
| `POST` | `/server/restart` | `server:manage` |
| `GET` | `/server/logs` | `server:manage` |
| `GET` | `/health` | публічно |

Перевірка в хендлері — один виклик:

```rust
// прикладний доступ — динамічне правило
rbac::require_permission(&state.db, auth.id, "mods:upload").await?;

// керування користувачами/ролями/правилами — тільки системний admin
rbac::require_admin(&state.db, auth.id).await?;
```

Нове правило в коді не з'являється саме по собі: якщо ендпоінту потрібен
`mods:publish`, це право треба створити через API (або додати в
`PERMISSION_CATALOG` у `src/rbac/service.rs`).

## Структура

```
src/
├── main.rs          # bootstrap: конфіг, БД, сідер, listener
├── app.rs           # Router, CORS, Swagger, /health
├── config.rs        # .env -> Config (fail fast)
├── db.rs            # пул + вбудовані міграції
├── error.rs         # AppError -> єдиний JSON { error, message }
├── openapi.rs       # OpenApi derive + bearer security scheme
├── pagination.rs    # ?limit=&offset= з LIMIT 1..100
├── seed.rs          # каталог правил, system-ролі, перший адмін
├── validation.rs    # ручна валідація DTO + тести
├── auth/            # jwt, argon2, extractor, routes
├── users/           # CRUD користувачів (admin), видача ролей/правил (admin)
├── rbac/            # permissions, roles, require_permission, require_admin
├── server/          # керування контейнером Minecraft через Portainer API
└── mods/            # файли MODS_DIR: dto, storage (шляхи/журнал), routes
migrations/
└── 0001_init.sql    # уся схема; права й system-ролі сідить boot-сідер
Dockerfile           # багатостадійна збірка образу (rust -> debian-slim, non-root)
.github/workflows/docker.yml  # CI: збірка й пуш образу в GHCR з main
```

## Docker

Образ збирається з `Dockerfile` і публікується в GHCR при кожному push у `main`:

```bash
docker pull ghcr.io/denisukvadim/minecraft_mods:latest

docker run -d --name minecraft-mods \
  -p 8080:8080 \
  -v minecraft-mods-data:/app/data \
  -e DATABASE_URL='postgres://user:pass@host:5432/minecraft_mods' \
  -e JWT_SECRET='<мінімум 32 символи>' \
  ghcr.io/denisukvadim/minecraft_mods:latest
```

Теги: `latest` (з `main`) і `sha-<commit>`. Змінні оточення — ті самі, що в
`.env.example`; `MODS_DIR` в образі вже вказує на `/app/data/mods`, тому том
`/app/data` зберігає завантажені моди між перезапусками.

Локальна збірка:

```bash
docker build -t minecraft-mods .
```

### Compose (production)

`docker-compose.prod.yml` піднімає API та адмінку за Traefik і використовує
зовнішні мережі `proxy` і `postgress_postgres` (`postgres`). Поруч із файлом має
лежати `.env` (Compose читає його автоматично):

```env
DOMAIN=example.com
DATABASE_URL=postgres://user:pass@postgres:5432/minecraft_mods
JWT_SECRET=<мінімум 32 символи>
PORTAINER_URL=https://portainer.example.com:9443
PORTAINER_API_KEY=ptr_...
MC_CONTAINER=minecraftserver

# необов'язково: імена піддоменів, теги образів, CORS, ADMIN_*, MODS_UID/GID
NAME=minecraft-mods-api
ADMIN_NAME=minecraft-mods-admin
```

```bash
docker compose -f docker-compose.prod.yml up -d
```

Що варто знати:

- тека модів монтується за тим самим абсолютним шляхом, що й `MODS_DIR`:
  `/home/veles/docker-config/minecraftserver/mods`. Створіть її заздалегідь і
  віддайте тому ж uid/gid, під яким працює Minecraft-сервер (типово `1000:1000`,
  змінні `MODS_UID`/`MODS_GID`);
- API стартує від непривілейованого користувача `1000` і пише файли з цими
  правами, щоб сервер Minecraft міг їх читати;
- роутери: `minecraft-mods-api.$DOMAIN` (API, `/health`) і
  `minecraft-mods-admin.$DOMAIN` (адмінка, `/`), сертифікати через
  `cloudflare`-resolver Traefik; middleware `default-headers@file` має існувати в
  конфігу Traefik;
- якщо адмінка деплоїться окремо, просто приберіть сервіс `admin` — API від того
  не залежить;
- Traefik healthcheck для API вказує на `/health` (а не `/`, який віддає 404).

## Перевірки

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Тести офлайн: хешування паролів, валідація, захист імен файлів, `Content-Disposition`,
наявність усіх шляхів і bearer-схеми в OpenAPI. E2E-тестів у `cargo test` немає —
сценарій перевірявся вручну на живій БД (див. нижче).

Перевірений вручну ланцюжок:

1. логін адміна → `get /auth/me` показує `mods:read/download/upload/delete`;
2. `POST /mods` (multipart) → `201`, файл з'явився в `MODS_DIR`;
3. `GET /mods` → файл у списку, `GET /mods/{name}/download` → той самий SHA-256;
4. `GET /mod-actions` → рядки `upload` і `download` з `user_id` і часом;
5. акаунт із роллю `user`: list/upload/download/delete `200`/`201`, сервер
   `200`, `GET /mods/archive` → `200 application/zip` з усіма файлами,
   `/mod-actions` → `200` завдяки `audit:read`;
6. він створює користувача без `roles` → `201`, новий акаунт має ті самі ролі
   й прямі права; спроба передати `roles` → `400`, підвищити доступ неможливо;
7. `DELETE /mods/{name}` → `204` і новий рядок у журналі; `../`, `%2F` у імені →
   `400`; повторний upload того ж імені → `409`;
8. `GET /mods/{name}/history` → `200` для `mods:read`, `username`/`user_email`
   приходять з `users` через join; для `../` — `400`;
9. після видалення файлу історія лишається; після видалення користувача
   `user_id` і разом з ним `username`/`user_email` стають `null`;
10. `PUT /auth/password`: хибний поточний пароль → `400`, короткий новий → `400`,
    успіх → `204`, старий пароль більше не пускає, новий пускає; старий JWT
    лишається дійсним до `exp`.

## Міграції

Файли лежать у `migrations/` і виконуються при старті. Для нової зміни схеми:

```bash
sqlx migrate add add_mod_tags   # якщо встановлено sqlx-cli
# або створити migrations/<timestamp>_add_mod_tags.sql вручну
```

Уже застосовані міграції не редагуються — SQLx зберігає їхні хеші.

## Обмеження, які варто знати

- JWT stateless: refresh-токенів і відкликання окремих access-токенів немає.
  Деактивація користувача діє негайно (перечитування з БД), а зміна пароля
  старі токени не відкликає — для цього знадобиться `token_version`.
- Кожен захищений запит робить один додатковий `EXISTS`-запит на перевірку
  права або ролі `admin`. Це ціна негайного відкликання; за потреби кешують
  права в Redis.
- Керування користувачами/ролями/правилами не можна делегувати окремим
  permission — тільки роль `admin`. Виняток — `POST /users` за правилом
  `users:create`, де новий акаунт отримує доступ творця без можливості обрати
  ролі. Якщо потрібні кілька адміністраторів, видайте їм роль `admin` явно.
- Права на файли не прив'язані до власника: `mods:delete` дозволяє видалити
  будь-який файл у теці. Розмежування «свій/чужий» потребувало б зберігати
  власника кожного файлу, а джерело істини тут — диск.
- Файли, покладені в `MODS_DIR` повз API (scp, ручне копіювання), не мають
  запису в журналі — журнал фіксує дії через API.
- Ім'я файлу обмежене 200 символами; не-UTF-8 імена не показуються в переліку
  і не доступні через API.
- Журнал не ротується. За потреби приберіть старі рядки
  (`DELETE FROM mod_file_actions WHERE created_at < now() - interval '90 days'`)
  або винесіть його в окрему таблицю/сховище.
- `CORS_ORIGINS=*` і `ADMIN_*` у `.env.example` — приклади. У проді задайте
  конкретні origin, унікальний `JWT_SECRET` і приберіть `ADMIN_PASSWORD`
  після створення адміністратора.
