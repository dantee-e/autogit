# Servidor

Python 3.11+, FastAPI, SQLAlchemy 2, Alembic. Código em `server/paddock_server/`.

## Executar em desenvolvimento

```bash
cd server
python -m venv .venv
.venv/bin/pip install -e '.[dev]'
PADDOCK_DATA=./data .venv/bin/paddock-server serve --reload
```

- `http://localhost:8000/`: configuração inicial (cria o primeiro administrador) e página de downloads.
- `http://localhost:8000/docs`: documentação interativa da API (Swagger), gerada pelo FastAPI.
- Testes: `.venv/bin/pytest` (usam um diretório temporário; não tocam em `./data`).

## Como uma requisição flui

```
HTTP ──▶ routers/<área>.py ──▶ deps.py (sessão do banco, usuário, permissão)
                │
                ▼
         services.py  (regras de negócio; lança errors.AppError)
                │
       ┌────────┼──────────────┐
       ▼        ▼              ▼
   models.py  storage.py   versioning.py
   (SQLite)   (blobs)      (rótulos)
                │
                ▼
         schemas.py (serializa para JSON) ──▶ HTTP
```

**Regra:** as rotas são finas. Toda decisão ("pode publicar?", "qual o próximo rótulo?") fica em
`services.py` ou `versioning.py`, para valer igual para o app, os scripts e os testes.

## Módulos

### `main.py`
`create_app(settings)` aplica as migrações pendentes (`db.upgrade`), cria o engine, guarda em
`app.state` o `settings`, o `sessionmaker`, o `BlobStore` e o `LoginThrottle`, registra os
roteadores e dois tratadores de erro:

- `AppError` → `{"detail": "<mensagem PT-BR>"}` com o código HTTP da subclasse.
- `RequestValidationError` (corpo/form inválido) → 422 com `detail` legível e `errors` detalhado.

O `uvicorn` chama `create_app` como *factory* (`paddock-server serve`).

### `config.py`
`Settings.from_env()` lê as variáveis `PADDOCK_*` (tabela no README principal) e cria as pastas
`data/`, `data/files/` e `data/downloads/`. Nos testes, `Settings.from_env(data_dir=tmp)`.

### `models.py`: modelo de dados

```
users ─┬─< api_tokens
       │
       ├─< projects ──< items ──< versions >──┐
       │                 │                     │
       │                 └─ locked_by ─▶ users │
       │                                       │
       └─< baselines ──< baseline_versions >───┘

activity  (log de tudo; referências opcionais a user/project/item/version)
```

| Tabela | Campos importantes | Observações |
|---|---|---|
| `users` | `username` (sempre minúsculo, único), `role` (`admin`/`member`/`viewer`), `active` | Usuários nunca são apagados, só desativados (o histórico continua apontando para eles). |
| `api_tokens` | `token_hash` (SHA-256), `name`, `expires_at` (nulo = não expira), `last_used_at` | Login do app cria um token com validade (`PADDOCK_TOKEN_DAYS`); tokens de CI não expiram. O token em si nunca é guardado. |
| `projects` | `kind` (`cad`/`software`), `subsystem`, `require_lock`, `archived` | Projeto arquivado vira somente leitura. |
| `items` | `name` (único no projeto), `part_number`, `file_stem`, `locked_by_id`, `locked_at`, `lock_note` | `file_stem` vem do nome do arquivo da 1ª versão e é único no projeto (sem diferenciar maiúsculas). |
| `versions` | `seq` (1, 2, 3…), `label` (`B`, `A.1`, `1.2.0`), `filename`, `sha256`, `size`, `comment`, `status` | `(item_id, seq)` e `(item_id, label)` são únicos. Conteúdo nunca muda; só `status` é editável. |
| `baselines` + `baseline_versions` | `name` (único no projeto) | Lista fixa de versões. |
| `activity` | `action`, `details` | Ações: `project_created`, `project_archived`, `project_restored`, `item_created`, `version_published`, `checkout`, `checkin`, `checkout_forced`, `status_changed` (`details` = `de->para`), `baseline_created`, `user_created`, `user_deactivated`. |

Datas são guardadas em UTC (`UTCDateTime` grava sem fuso e devolve sempre com `tzinfo=UTC`,
porque o SQLite descarta fuso).

### `versioning.py`
Funções puras, sem banco:

- `next_label(kind, previous, bump)`:
  - **software** (`patch`/`minor`/`major`): semver; a primeira versão parte de `0.0.0`
    (`minor` → `0.1.0`, `major` → `1.0.0`).
  - **cad** (`revision`/`iteration`): letras `ABCDEFGHJKLMNPRTUVWY` (sem I, O, Q, S, X, Z,
    conforme ASME Y14.35), numeração bijetiva (`Y` → `AA` → `AB`…); iteração acrescenta `.N`
    (`B` → `B.1` → `B.2`); nova revisão descarta a iteração (`B.2` → `C`).
- `previews(kind, previous)`: próximo rótulo para cada tipo de incremento; o cliente mostra
  isso no diálogo de publicação.
- `STATUSES`, `PRIVILEGED_STATUSES` (`approved`, `released`), `BUMPS`, `DEFAULT_BUMP`.

### `services.py`: regras de negócio

| Função | Regras |
|---|---|
| `create_project` | Nome e subsistema obrigatórios; nome único (sem diferenciar maiúsculas). |
| `create_item` | Projeto não arquivado; nome único no projeto; **nome-base do arquivo único no projeto** (as cópias locais ficam lado a lado numa pasta); cria item + 1ª versão na mesma transação. |
| `add_version` | Projeto não arquivado; se o item está em check-out com **outra** pessoa → 403; se o projeto exige check-out e o item não está com você → 403. |
| `_publish` (interna) | Limpa o nome do arquivo (`clean_filename`); comentário obrigatório; tipo de incremento válido; grava o blob; rejeita arquivo vazio (400), maior que o limite (400) ou **idêntico à versão atual** (409); define `file_stem` na 1ª versão; libera o check-out de quem publicou, a menos que `keep_lock`; conflito de `seq` em publicação simultânea → 409. |
| `lock_item` | `UPDATE … WHERE locked_by_id IS NULL` atômico: se duas pessoas clicam juntas, só uma ganha; a outra recebe 409 com o nome de quem bloqueou. |
| `unlock_item` | Quem bloqueou pode liberar; administrador pode **forçar** (registrado como `checkout_forced`, com o nome da pessoa no histórico). |
| `set_status` | Status válido; projeto não arquivado; **mudar para ou a partir de** `approved`/`released` exige administrador. |
| `create_baseline` | Nome único no projeto; pega a última versão de cada item (ou a última `released`, se `only_released`); erro se nenhuma versão for elegível. |
| `clean_filename` | Remove caminho, caracteres inválidos no Windows (`<>:"|?*`), pontos/espaços finais e prefixa nomes reservados (`CON`, `PRN`, `COM1`…). |
| `item_file_name` / `download_name` | Nome na pasta de trabalho: `<file_stem><extensão da versão>`. Nome de download: `<stem>_rev<rótulo><ext>` (CAD) ou `<stem>_v<rótulo><ext>` (software). |

As funções recebem a sessão do banco e fazem `commit` ao final; em erro de validação depois de
começar a escrever, fazem `rollback` antes de lançar a exceção.

### `storage.py`
`BlobStore` grava cada arquivo em `files/<2 primeiros hex>/<sha256>`. O upload é lido em blocos
de 1 MB, com hash calculado durante a escrita num arquivo temporário. Se o blob já existe, o
temporário é descartado (deduplicação). Blobs **nunca** são apagados nem alterados.
`verify(sha)` recalcula o hash (usado por `GET /api/items/{id}/verify`).

### `security.py`
- Senhas: `hashlib.scrypt` (N=2¹⁴, r=8, p=1, sal aleatório de 16 bytes), formato
  `scrypt$N$r$p$sal$hash`. Sem dependências externas.
- Tokens: `pdk_` + 32 bytes aleatórios; no banco fica só o SHA-256.
- `LoginThrottle`: 5 falhas em 5 minutos para o mesmo par usuário+IP bloqueiam novas tentativas
  (429). Fica em memória (zera ao reiniciar o servidor).

### `deps.py`
Dependências injetadas pelo FastAPI:

- `get_db`: uma sessão SQLAlchemy por requisição.
- `current_token` / `current_user`: lê `Authorization: Bearer`, rejeita token inexistente,
  expirado ou de usuário desativado (401); atualiza `last_used_at` no máximo uma vez por minuto.
- `writer`: `admin` ou `member`. `admin`: só `admin`. Respondem 403 com mensagem em PT-BR.

### `errors.py`
`Invalid` 400 · `Unauthorized` 401 · `Forbidden` 403 · `NotFound` 404 · `Conflict` 409 ·
`TooManyRequests` 429. Sempre com `message` pronta para o usuário.

### `schemas.py`
Modelos Pydantic de entrada (`*In`) e saída (`*Out`), mais funções `*_out(db, obj)` que montam
as respostas (ex.: `item_out` inclui a última versão, a contagem de versões e os
`next_labels`). Ver [api.md](api.md).

### `web.py`
HTML mínimo sem template engine (escapado com `html.escape`):

- `GET /`: formulário de configuração inicial se não há usuários; senão, lista os arquivos de
  `data/downloads/` (para distribuir o app para a equipe).
- `POST /setup`: cria o primeiro administrador. Só funciona enquanto não existe nenhum usuário.
- `GET /downloads/{arquivo}`: serve arquivos dessa pasta, bloqueando `..` e subpastas.

### `__main__.py`
CLI `paddock-server`:
- `serve [--host] [--port] [--reload]`
- `create-admin <usuario> [--name "Nome"]` (pede a senha no terminal)

## Migrações (Alembic)

- Ficam em `paddock_server/migrations/versions/`. A primeira é `0001_initial_schema.py`.
- São aplicadas **automaticamente** a cada start (`db.upgrade`), e também pelo `create-admin`.
- `render_as_batch=True` permite `ALTER TABLE` no SQLite.
- Para criar uma migração depois de mudar `models.py`:

  ```bash
  cd server
  .venv/bin/python -c "from alembic import command; from paddock_server.db import alembic_config; \
  command.revision(alembic_config('sqlite:///data/paddock.sqlite3'), message='adiciona campo x', autogenerate=True)"
  ```

  Revise o arquivo gerado. Prefira tipos genéricos do SQLAlchemy (`sa.DateTime()`), sem importar
  `paddock_server.models`, para a migração não quebrar quando o modelo mudar no futuro.

## Banco PostgreSQL (opcional)

```bash
.venv/bin/pip install "psycopg[binary]"
PADDOCK_DATABASE_URL=postgresql+psycopg://usuario:senha@host/paddock .venv/bin/paddock-server serve
```

Os arquivos continuam em `PADDOCK_DATA/files`. As comparações sem diferenciar maiúsculas usam
`func.lower`, que funciona nos dois bancos.

## Segurança e operação

- Coloque o servidor atrás de HTTPS (Caddy/nginx) se ele sair da rede da oficina. O uvicorn
  roda com `proxy_headers=True`, então o IP real chega ao limitador de login.
- **Backup:** pare o servidor e copie `PADDOCK_DATA` inteira (banco + `files/`). Com SQLite em
  modo WAL, copiar com o servidor rodando pode pegar o banco no meio de uma escrita.
- Usuários desativados perdem todas as sessões e têm os check-outs liberados.
- Redefinir a senha de alguém derruba todas as sessões dessa pessoa.

## Testes

`server/tests/test_api.py` cobre as regras de rótulo, a limpeza de nomes, autenticação e
limitador, o fluxo CAD com check-out, rejeições (idêntico, vazio, sem comentário),
permissões de status, liberação forçada e desativação, congelamento com zip, token de CI,
busca e verificação de integridade, proteção contra `..` na pasta de downloads e nomes de
arquivo duplicados no projeto.

Novos testes seguem o padrão: fixture `admin` (faz o setup e devolve um cliente autenticado),
helpers `make_user`, `new_project`, `new_item`, `publish`.
