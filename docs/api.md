# Referência da API

Base: `http://<servidor>:8000`. Todas as rotas `/api/*` falam JSON, exceto uploads
(`multipart/form-data`) e downloads (binário). A documentação interativa gerada pelo FastAPI
fica em `/docs` (Swagger) e `/redoc`.

## Autenticação

```
Authorization: Bearer pdk_xxxxxxxxxxxxxxxx
```

- **Sessão do app:** `POST /api/auth/login` devolve um token que expira em `PADDOCK_TOKEN_DAYS`
  dias (padrão 90).
- **Token de script/CI:** `POST /api/auth/tokens` cria um token que **não expira**, com as mesmas
  permissões do usuário que o criou. Revogue com `DELETE /api/auth/tokens/{id}`.

Coluna **Acesso** nas tabelas abaixo: *público* (sem token), *logado* (qualquer papel),
*escrita* (`admin` ou `member`), *admin*.

## Erros

Todo erro tem o formato:

```json
{"detail": "Este item está em check-out com João Pereira."}
```

| Código | Quando |
|---|---|
| 400 | Dado inválido (comentário vazio, tipo de incremento errado, arquivo vazio ou grande demais) |
| 401 | Sem token, token inválido/expirado, usuário desativado, senha errada no login |
| 403 | Sem permissão para a ação (papel, check-out de outra pessoa, projeto arquivado) |
| 404 | Recurso não existe |
| 409 | Conflito: nome duplicado, item já em check-out, arquivo idêntico à versão atual, publicação simultânea |
| 422 | Corpo/formulário malformado (campo obrigatório faltando). Vem com `errors` detalhado. |
| 429 | Muitas tentativas de login (5 falhas em 5 min) |

## Valores fixos

| Campo | Valores |
|---|---|
| `role` | `admin`, `member`, `viewer` |
| `kind` (projeto) | `cad`, `software` |
| `status` (versão) | `draft`, `in_review`, `approved`, `released`, `obsolete` |
| `bump` (CAD) | `revision` (padrão), `iteration` |
| `bump` (software) | `patch`, `minor` (padrão), `major` |

Datas em ISO 8601 UTC (`2026-11-05T13:51:02Z`).

---

## Meta

| Método | Rota | Acesso | Descrição |
|---|---|---|---|
| GET | `/api/meta` | público | Nome da equipe, versão do servidor, subsistemas, status, tipos de incremento, `has_users` |

```json
{"team_name": "Equipe Tupã Racing", "server_version": "0.2.0",
 "subsystems": ["Chassi", "Suspensão", "..."], "statuses": ["draft", "..."],
 "bumps": {"software": ["patch", "minor", "major"], "cad": ["revision", "iteration"]},
 "min_password": 8, "has_users": true}
```

## Autenticação: `/api/auth`

| Método | Rota | Acesso | Corpo | Resposta |
|---|---|---|---|---|
| POST | `/api/auth/login` | público | `{"username", "password", "device_name"?}` | `{"token", "expires_at", "user"}` |
| POST | `/api/auth/logout` | logado | | 204. Revoga o token usado na chamada. |
| GET | `/api/auth/me` | logado | | Usuário atual |
| PUT | `/api/auth/password` | logado | `{"current", "new"}` | 204 (403 se a senha atual estiver errada) |
| GET | `/api/auth/tokens` | logado | | Tokens/sessões do usuário (`current: true` marca o da chamada) |
| POST | `/api/auth/tokens` | logado | `{"name"}` | 201, `{..., "token"}`. **O token só aparece nesta resposta.** |
| DELETE | `/api/auth/tokens/{id}` | logado | | 204 |

`username` não diferencia maiúsculas. `device_name` aparece na lista de sessões (o app manda
`Paddock desktop (<computador>, <sistema>)`).

## Usuários: `/api/users`

| Método | Rota | Acesso | Corpo | Observações |
|---|---|---|---|---|
| GET | `/api/users/directory` | logado | | `[{id, name}]` dos usuários ativos |
| GET | `/api/users` | admin | | Lista completa (inclui desativados) |
| POST | `/api/users` | admin | `{"username", "name", "password", "role", "subsystem"?}` | 201. Usuário: 2–64 caracteres `a-z 0-9 . _ -`. Senha: mín. 8. |
| PATCH | `/api/users/{id}` | admin | qualquer subconjunto de `{"name", "role", "subsystem", "active", "password"}` | `active: false` revoga sessões e libera check-outs; `password` revoga sessões. O admin não pode mudar o próprio papel nem se desativar. |

Objeto usuário: `{id, username, name, role, subsystem, active, created_at}`.

## Projetos: `/api/projects`

| Método | Rota | Acesso | Corpo / parâmetros | Observações |
|---|---|---|---|---|
| GET | `/api/projects` | logado | `?include_archived=true` | Ordenados por arquivado, subsistema, nome |
| POST | `/api/projects` | escrita | `{"name", "kind", "subsystem", "description"?, "require_lock"?}` | 201. `require_lock` padrão: `true` para CAD, `false` para software. |
| GET | `/api/projects/{id}` | logado | | |
| PATCH | `/api/projects/{id}` | admin | `{"description"?, "require_lock"?, "archived"?}` | |

Objeto projeto: `{id, name, kind, subsystem, description, require_lock, archived, item_count, last_update, created_at}`.

## Itens

| Método | Rota | Acesso | Corpo / parâmetros | Observações |
|---|---|---|---|---|
| GET | `/api/projects/{id}/items` | logado | | Ordenados por nº de peça, depois nome |
| POST | `/api/projects/{id}/items` | escrita | **multipart**: `name`, `comment`, `file`, `bump`?, `part_number`?, `description`? | 201. Cria o item **e** a 1ª versão. 409 se o nome ou o nome-base do arquivo já existir no projeto. |
| GET | `/api/items/{id}` | logado | | |
| POST | `/api/items/{id}/lock` | escrita | `{"note"?}` | Check-out. 409 se já bloqueado. |
| DELETE | `/api/items/{id}/lock` | escrita | | Libera. De outra pessoa: só admin (forçado). |
| GET | `/api/items/{id}/verify` | logado | | `{"ok": true, "corrupted": []}`: recalcula o SHA-256 de todas as versões |

Objeto item:

```json
{
  "id": 1, "project_id": 1, "project_name": "Suspensão FS-26", "project_kind": "cad",
  "require_lock": true, "name": "Manga de eixo dianteira", "part_number": "SUS-0142",
  "description": "", "file_name": "manga.dwg",
  "locked_by": {"id": 2, "name": "João Pereira"}, "locked_at": "2026-11-05T13:51:02Z",
  "lock_note": "furação da pinça",
  "latest_version": { "...": "objeto versão" },
  "version_count": 3,
  "next_labels": {"revision": "D", "iteration": "C.1"}
}
```

`file_name` é o nome que a cópia local deve ter (nome-base fixo + extensão da última versão).

## Versões

| Método | Rota | Acesso | Corpo / parâmetros | Observações |
|---|---|---|---|---|
| GET | `/api/items/{id}/versions` | logado | | Da mais nova para a mais antiga |
| POST | `/api/items/{id}/versions` | escrita | **multipart**: `file`, `comment`, `bump`?, `keep_lock`? (`true`/`false`) | 201. Publica. Libera o check-out de quem publicou, a menos que `keep_lock=true`. |
| PATCH | `/api/versions/{id}` | escrita | `{"status"}` | Para ou a partir de `approved`/`released`: só admin |
| GET | `/api/versions/{id}/download` | logado | | Binário |
| GET | `/api/items/{id}/versions/{rotulo}/download` | logado | `rotulo` = `B`, `1.2.0`… ou `latest`; `?status=released` opcional | Binário. 404 se não houver versão com esse critério. |

Objeto versão:

```json
{"id": 7, "item_id": 1, "seq": 3, "label": "C", "filename": "manga.dwg",
 "sha256": "6978d187…", "size": 70012, "comment": "Raio de concordância R5",
 "status": "draft", "author": {"id": 2, "name": "João Pereira"},
 "created_at": "2026-11-05T13:51:40Z", "baselines": [{"id": 1, "name": "Design Review 1"}]}
```

Cabeçalhos dos downloads:

| Cabeçalho | Conteúdo |
|---|---|
| `Content-Disposition` | `attachment; filename=manga_revC.dwg` |
| `X-Paddock-Version` | Rótulo (`C`) |
| `X-Paddock-Version-Id` | ID da versão |
| `X-Paddock-SHA256` | Hash do conteúdo (o app confere depois de baixar) |

## Congelamentos

| Método | Rota | Acesso | Corpo | Observações |
|---|---|---|---|---|
| GET | `/api/projects/{id}/baselines` | logado | | Do mais novo para o mais antigo |
| POST | `/api/projects/{id}/baselines` | admin | `{"name", "description"?, "only_released"?}` | 201, devolve o detalhe |
| GET | `/api/baselines/{id}` | logado | | `{..., "entries": [{item_id, item_name, part_number, file_name, download_name, version}]}` |
| GET | `/api/baselines/{id}/zip` | logado | | Zip com uma pasta por item e um `MANIFESTO.txt` |

## Atividade, revisões, busca

| Método | Rota | Acesso | Parâmetros | Observações |
|---|---|---|---|---|
| GET | `/api/activity` | logado | `project_id`?, `item_id`?, `limit` (1–500, padrão 50) | Mais recente primeiro. Ações listadas em [servidor.md](servidor.md#modelspy-modelo-de-dados). |
| GET | `/api/reviews` | logado | | Versões `in_review` de projetos ativos, com `item_name`, `project_id`, `project_name`, `project_kind` |
| GET | `/api/locks` | logado | | IDs dos itens em check-out com quem chama |
| GET | `/api/search` | logado | `q` | `{"items": [...], "versions": [...]}`. Busca em nome, nº de peça, descrição, comentário, nome de arquivo; `q` igual a um SHA-256 acha a versão exata. |

## Páginas HTML (fora da API)

| Rota | Descrição |
|---|---|
| `GET /` | Setup inicial (sem usuários) ou página com links de download do app |
| `POST /setup` | Formulário: `name`, `username`, `password`. Só funciona sem usuários. |
| `GET /downloads/{arquivo}` | Arquivos de `PADDOCK_DATA/downloads/` |

---

## Exemplos

### Publicar firmware a partir do CI (GitHub Actions)

```yaml
- name: Publicar firmware no Paddock
  env:
    PADDOCK_TOKEN: ${{ secrets.PADDOCK_TOKEN }}
  run: |
    curl --fail-with-body -H "Authorization: Bearer $PADDOCK_TOKEN" \
      -F file=@build/ecu.hex -F bump=patch \
      -F comment="Build ${GITHUB_SHA::7}: ${{ github.event.head_commit.message }}" \
      https://paddock.equipe.com.br/api/items/4/versions
```

### Baixar a última versão liberada (gravar no carro)

```bash
curl -fOJ -H "Authorization: Bearer $PADDOCK_TOKEN" \
  "https://paddock.equipe.com.br/api/items/4/versions/latest/download?status=released"
```

### Python

```python
import httpx

api = httpx.Client(base_url="http://servidor:8000",
                   headers={"Authorization": "Bearer pdk_..."})
item = api.get("/api/items/1").json()
print(item["latest_version"]["label"], item["locked_by"])

api.post("/api/items/1/lock", json={"note": "ajuste do furo"}).raise_for_status()
with open("manga.dwg", "rb") as f:
    v = api.post("/api/items/1/versions",
                 data={"comment": "Furo de 10 mm", "bump": "revision"},
                 files={"file": ("manga.dwg", f)}).json()
print("publicado", v["label"])
```
