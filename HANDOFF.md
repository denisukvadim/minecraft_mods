# HANDOFF — бекенд `minecraftmods_backend`

> Оновлено: **17.11.2026**. Це документ передачі: поточний стан, ухвалені рішення,
> пастки середовища й план подальшої доробки. Загальне використання API описане в
> [`README.md`](README.md); тут — те, що потрібно знати, щоб **продовжити роботу**.

## 1. Що це

REST API файлового каталогу модів для Minecraft: JWT-аутентифікація, динамічна
рольова система (permissions — рядки в БД, створюються в рантаймі) і журнал дій з
файлами. Каталог модів — **вміст папки `MODS_DIR`**, а не таблиця в БД. Адмінка
лежить окремо: [`../admin-panel`](../admin-panel).

Стек: Rust 2024, Tokio, Axum 0.8, PostgreSQL + SQLx 0.9 (міграції вбудовані),
utoipa 6 + Swagger UI, JWT HS256, Argon2id.

## 2. Стан на момент передачі

**Готово й перевірено:**

- аутентифікація: `POST /auth/login`, `GET /auth/me`, JWT, Argon2id;
- динамічний RBAC: permissions, роли, прямі видачі користувачу, `require_permission`;
- керування користувачами/ролями/правилами — лише системна роль `admin`
  (`require_admin`, перевірка імені ролі, не permission);
- моди на диску: list / upload (multipart, атомарний запис) / download / delete;
- журнал `mod_file_actions` + `GET /mod-actions` і `GET /mods/{file_name}/history`
  (тільки admin), ідентичність через `LEFT JOIN users`;
- Swagger UI на `/docs`, OpenAPI JSON на `/api-docs/openapi.json`, `/health`;
- міграції `0001`–`0005`, seed каталогу правил і system-ролей, bootstrap-адмін;
- `cargo test --quiet` — **11/11 passed** (хешування, валідація, імена файлів,
  `Content-Disposition`, наявність шляхів і bearer-схеми в OpenAPI);
- ручний E2E на живій БД (логін, CRUD, upload/download/delete, 403 для не-адміна,
  негайна дія зміни ролей, історія після видалення файлу/користувача).

**Жива БД (важливо):**

- міграції `0001`–`0005` **уже застосовані**; редагувати застосовані файли не можна
  (SQLx зберігає хеші);
- у `data/mods` лежить реальний файл користувача
  `trackwork-1.20.1-1.2.4.jar` (577 556 байт) — **не видаляти** під час тестів;
- у `mod_file_actions` є рядок `id = 7` (upload цього файлу) — **не видаляти**;
- роль `user` у цій конкретній БД має лише `mods:read`: seed додає
  `mods:download` тільки новоствореній ролі. Якщо потрібно — видати через
  `PUT /roles/{id}/permissions`;
- `.env` містить реальні креденшели до зовнішнього PostgreSQL. Не комітити, не
  вставляти в документи/логи.

**Стан процесів:** порти `8080` і `4200` вільні; фонові процеси зупинені.

**Git:** батьківська тека не є git-репозиторієм. Історії змін немає. Перед
подальшою роботою варто ініціалізувати git і зробити перший коміт (`.env` — у
`.gitignore`).

## 3. Швидкий старт

```bash
cd ~/work/personal/backend/minecraftmods_backend

cp .env.example .env          # заповнити DATABASE_URL, JWT_SECRET, ADMIN_*
docker compose up -d db       # локальний PostgreSQL 18 (або свій DATABASE_URL)
cargo run                     # http://localhost:8080
```

Міграції застосовуються автоматично в `db::connect` (`sqlx::migrate!`). Окремого
`sqlx-cli` не потрібно.

Точки входу:

- API: `http://localhost:8080/api/v1`
- Swagger: `http://localhost:8080/docs`
- OpenAPI: `http://localhost:8080/api-docs/openapi.json`
- Health: `http://localhost:8080/health`

Корисні команди:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
RUST_LOG=debug cargo run
```

## 4. Архітектурна карта

```
src/
├── main.rs        # dotenv → Config → DB+migrations → seed → Router → serve
├── app.rs         # Router, CORS, DefaultBodyLimit для mods, Swagger, /health
├── config.rs      # .env → Config, fail-fast валідація, канонікалізація MODS_DIR
├── db.rs          # PgPool (max 10) + sqlx::migrate!
├── error.rs       # AppError → єдиний JSON { error, message }
├── openapi.rs     # ApiDoc, bearer security scheme
├── pagination.rs  # ?limit=&offset=, limit clamp 1..100, offset >= 0
├── seed.rs        # каталог правил, system-роли, опційний перший адмін
├── validation.rs  # ручна валідація email/username/password/role/permission
├── auth/
│   ├── dto.rs         # LoginRequest, TokenResponse, MeResponse
│   ├── extractor.rs   # AuthUser: перечитує users з БД на кожен запит
│   ├── jwt.rs         # encode/decode HS256, exp
│   ├── password.rs    # Argon2id hash/verify (spawn_blocking)
│   └── routes.rs      # login, me
├── users/
│   ├── dto.rs         # UserSummary/Detail, Create/Update
│   ├── service.rs     # load_user_summary/detail
│   └── routes.rs      # CRUD users + assign/revoke role/permission
├── rbac/
│   ├── dto.rs         # PermissionDto, RoleDto/Summary, AccessResponse
│   ├── service.rs     # PERMISSION_CATALOG, require_permission, require_admin
│   └── routes.rs      # permissions CRUD, roles CRUD, PUT role permissions
└── mods/
    ├── dto.rs         # ModFileDto, ModFileActionDto, UploadModForm
    ├── storage.rs     # валідація імені, resolve, list, atomic upload, log_action
    └── routes.rs      # list/upload/download/delete/history/mod-actions
```

Потік запиту: `Router` → handler → `AuthUser` (перечитує користувача з БД) →
`rbac::require_*` (один `EXISTS`-запит) → робота з диском/БД → `AppError` у
форматі `{ "error": "...", "message": "..." }`.

## 5. Схема БД

Актуальні таблиці після `0005`:

| Таблиця | Призначення |
|---|---|
| `users` | акаунти: `email`/`username` UNIQUE, `password_hash`, `is_active`, timestamps |
| `permissions` | правила: `code` UNIQUE у форматі `resource:action` |
| `roles` | ролі: `name` UNIQUE, `is_system` (системні не видаляються) |
| `role_permissions` | M:N роль ↔ правило, каскадне видалення |
| `user_roles` | M:N користувач ↔ роль |
| `user_permissions` | прямі видачі користувачу + `granted_at` |
| `mod_file_actions` | журнал: `id`, `user_id` (`ON DELETE SET NULL`), `action`, `file_name`, `created_at` |

Таблиці `mods` (метадані) немає: джерело істини — диск. У `mod_file_actions`
**немає** `username`/`user_email` — вони беруться join-ом.

Індекси: `created_at DESC, id DESC` і `file_name` у журналі, `user_id`;
зворотні індекси для M:N таблиць.

Міграції:

```
0001_init.sql   # уся схема: users, permissions, roles, M:N таблиці, mod_file_actions
```

Каталог прав і system-ролі (`admin`, `user`) створює сідер при старті
(`src/seed.rs`), тому в міграції їх немає.

**Правило:** для зміни схеми створювати новий файл `migrations/0002_*.sql`
(вручну або `sqlx migrate add`). Застосовані файли не редагувати.

## 6. API

| Метод | Шлях | Доступ |
|---|---|---|
| `POST` | `/api/v1/auth/login` | публічно |
| `GET` | `/api/v1/auth/me` | будь-який автентифікований |
| `GET` | `/api/v1/mods` | `mods:read` (пагінація) |
| `POST` | `/api/v1/mods` | `mods:upload` (multipart, поле `file`) |
| `GET` | `/api/v1/mods/{file_name}/download` | `mods:download` |
| `DELETE` | `/api/v1/mods/{file_name}` | `mods:delete` |
| `GET` | `/api/v1/mods/{file_name}/history` | **admin** (пагінація) |
| `GET` | `/api/v1/mod-actions` | **admin** (пагінація) |
| `POST` | `/api/v1/users` | **admin** |
| `GET` | `/api/v1/users`, `/users/{id}` | `users:read` |
| `PATCH`/`DELETE` | `/api/v1/users/{id}` | **admin** |
| `POST`/`DELETE` | `/api/v1/users/{id}/roles/{role_id}` | **admin** |
| `POST`/`DELETE` | `/api/v1/users/{id}/permissions/{permission_id}` | **admin** |
| `GET` | `/api/v1/permissions` | `permissions:read` |
| `POST`/`DELETE` | `/api/v1/permissions`, `/permissions/{id}` | **admin** |
| `GET` | `/api/v1/roles`, `/roles/{id}` | `roles:read` |
| `POST`/`PATCH`/`DELETE` | `/api/v1/roles`, `/roles/{id}` | **admin** |
| `PUT` | `/api/v1/roles/{id}/permissions` | **admin** |
| `GET` | `/health` | публічно |

Особливості контрактів:

- list-ендпоінти повертають **масив без total**; пагінація `?limit=&offset=`,
  `limit` клампиться до 1..100, offset ≥ 0. Адмінка тому запитує `limit + 1`;
- помилки завжди `{ error, message }`; статуси — 400/401/403/404/409/413/500;
- ім'я файлу — один сегмент: без `/`, `\`, `..`, керуючих символів, не починається
  з `.`, максимум 200 символів; symlink не обслуговується;
- upload атомарний (`.upload-<uuid>` → rename), існуюче ім'я → `409`, ліміт
  `MODS_MAX_UPLOAD_BYTES` → `413`;
- download завжди `application/octet-stream`, `Content-Disposition: attachment`,
  `X-Content-Type-Options: nosniff`; історія для невідомого файлу → `200 []`.

## 7. Ключові рішення (чому так)

- **Керування — перевірка ролі `admin`, а не permission.** Інакше роль із правом
  на редагування правил могла б видати собі будь-що (privilege escalation).
  Динамічні permissions лишилися для прикладного доступу (моди, читання довідників).
- **Seed щостарту:** каталог правил і роль `admin` відновлюються завжди; роль `user`
  отримує `mods:read` + `mods:download` лише при створенні, щоб адміністратор міг
  забрати право без відновлення після рестарту.
- **Журнал не дублює ідентичність.** `user_id` + `LEFT JOIN users` дає актуальні
  username/email; після видалення акаунта join дає `NULL` («deleted account»).
  Хто потребує вічної атрибуції — деактивує акаунт, а не видаляє.
- **Файли — джерело істини.** Немає таблиці метаданих; список читається з диска.
  Файли, покладені повз API, не мають запису в журналі.
- **Негайне відкликання доступу.** `AuthUser` і перевірки читають БД на кожен
  запит; ціна — додатковий `EXISTS`-запит. Кешування (Redis) — можлива оптимізація.
- **Паролі:** Argon2id у `spawn_blocking`, щоб не блокувати async-рантайм.
- **CORS:** `CORS_ORIGINS=*` для розробки; у проді — конкретний origin адмінки.

## 8. Відомі обмеження / технічний борг

- JWT stateless: немає refresh-токенів і відкликання окремих access-токенів. Після
  зміни пароля старі токени живуть до `exp` (потрібен `token_version`).
- Немає інтеграційних/E2E тестів у `cargo test` — лише unit; сценарії перевірялися
  вручну.
- Кожен захищений запит робить додатковий `EXISTS`; при навантаженні знадобиться
  кеш або денормалізація.
- Журнал не ротується і не має TTL/архіву.
- Права на файли не прив'язані до власника: `mods:delete` дозволяє видалити будь-який
  файл.
- List-ендпоінти не віддають total і не підтримують пошук/сортування.
- Немає rate limiting, request-id, метрик, readiness-перевірки окрім `/health`.
- Імена файлів >200 символів і не-UTF-8 не обслуговуються.
- Повторний upload того самого імені — `409`, окремого «replace» немає.

## 9. Пропозиції для подальшої доробки

1. **E2E-тести** (`#[sqlx::test]` або testcontainers + окрема тестова БД): логін,
   403-матриця, upload/download/delete, історія, каскади. Найбільша прогалина.
2. **`token_version` у `users`** + інкремент при зміні пароля/деактивації; перевірка
   у `AuthUser`. Разом із цим — refresh-токени, якщо потрібна довга сесія.
3. **Ретенція журналу**: періодичне видалення/архівування старих рядків, індекс уже є.
4. **Сервіс `api` у compose** для локального запуску всього стека (Dockerfile уже є,
   образ публікується в GHCR з `main`).
5. **Метадані модів** (версія, опис, теги) — або sidecar-файл, або нова таблиця;
   поточний контракт свідомо мінімальний.
6. **Власник файлу** і права «свій/чужий», якщо з'явиться багато видавців.
7. **Пошук/сортування/total** для list-ендпоінтів (зараз пагінація offset-only).
8. **Rate limiting** на `/auth/login` і upload; request-id у логах.
9. **Метрики/трейсинг** (OpenTelemetry) і окремий readiness.
10. **Розширити OpenAPI-приклади** й згенерувати клієнт для адмінки замість ручних
    типів, якщо контракт почне рости.

## 10. Як перевіряти після змін

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo run
```

Мінімальний ручний сценарій (є в `README.md` детальніше):

1. логін адміна → `/auth/me` містить `mods:read/download/upload/delete`;
2. upload нового файлу → `201`, файл з'явився в `MODS_DIR`, у журналі рядок `upload`;
3. list + download → SHA-256 збігається; у журналі `download`;
4. не-адмін: list `200`, upload/delete `403`, журнал `403`;
5. видати роль/право → доступ діє без перелогіну;
6. `DELETE /mods/{name}` → `204`, файл зник, історія лишилася;
7. `../`, `%2F`, приховані імена → `400`; повторний upload → `409`;
8. видалити користувача → у журналі `user_id`/ім'я стають `NULL`.

**Не робити під час тестів:** не видаляти `trackwork-1.20.1-1.2.4.jar` і рядок
журналу `id = 7`, не застосовувати `cargo sqlx migrate revert` на живій БД, не
змінювати застосовані міграції.

## 11. Пастки середовища

- `MODS_DIR` канонікалізується на старті: зміна шляху в `.env` вимагає рестарту.
- `.env` містить реальні секрети; у прикладах і звітах їх бути не повинно.
- `ADMIN_*` потрібні лише для першого створення адміна; після цього їх варто
  прибрати з `.env`.
- `CORS_ORIGINS=*` небезпечний у проді — задайте origin адмінки.
- Роль `user` у живій БД не має `mods:download` (див. розділ 2) — це не баг коду,
  а наслідок seed-логіки «тільки при створенні».
- Видалення користувача незворотно анонімізує його історію.
- `/docs` змонтований поза `/api/v1`; проксі адмінки має вести і `/docs`.

## 12. Чеклист «перший день» після повернення

- [ ] переконатися, що `.env` на місці й БД доступна (`/health`);
- [ ] `cargo test` — 11/11;
- [ ] `cargo run`, відкрити `/docs`, залогінитися адміном;
- [ ] перевірити, що `data/mods` і рядок журналу `id = 7` на місці;
- [ ] ініціалізувати git (за потреби) і закомітити поточний стан;
- [ ] вибрати пункт із розділу 9 і почати з тестів/міграції, а не з UI.
