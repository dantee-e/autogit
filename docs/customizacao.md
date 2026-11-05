# Receitas de customização

Passo a passo das mudanças mais comuns. Depois de qualquer mudança, rode os testes:
`cd server && .venv/bin/pytest` e `cd client && cargo test`.

## Sem mexer em código

| O quê | Como |
|---|---|
| Nome da equipe | `PADDOCK_TEAM_NAME="Equipe X"` no servidor |
| Subsistemas | `PADDOCK_SUBSYSTEMS="Chassi,Suspensão,Powertrain,Eletrônica"` |
| Tamanho máximo de arquivo | `PADDOCK_MAX_UPLOAD_MB=2048` (lembre também do limite do proxy HTTPS) |
| Validade das sessões do app | `PADDOCK_TOKEN_DAYS=30` |
| Check-out obrigatório por projeto | No app: projeto → **Configurar** (admin) |

## Mudar a identidade visual

- **Cores:** `client/ui/globals.slint` → `Theme` (`brand`, `ok`, `warn`, `danger`, `info`).
- **Ícone:** troque `client/assets/icon-1024.png`, `icon-256.png` e `icon.ico` (o `.ico` precisa
  ter vários tamanhos; ex.: `convert icon-1024.png -define icon:auto-resize=256,128,64,48,32,16 icon.ico`).
- **Nome do app:** título da janela em `ui/app.slint` (`AppWindow.title`), textos "Paddock" em
  `ui/views.slint` (login) e `ui/app.slint` (barra superior), `Info.plist` (macOS),
  `paddock.desktop` e `[package.metadata.deb]` (Linux).

## Mudar o intervalo de atualização automática

`client/src/main.rs`: `Duration::from_secs(60)`.

## Mudar o esquema de revisões CAD

Tudo em `server/paddock_server/versioning.py`.

- **Letras permitidas:** `REV_LETTERS`. Para usar o alfabeto inteiro:
  `REV_LETTERS = "ABCDEFGHIJKLMNOPQRSTUVWXYZ"`.
- **Primeira revisão diferente de "A"** (ex.: começar em `0` ou `-` antes da liberação): ajuste
  o ramo `if not previous:` de `next_label` e a expressão regular `CAD_RE` para aceitar o novo
  formato.
- **Iterações com outro separador** (`B-1` em vez de `B.1`): mude o `f"{letters}.{iteration + 1}"`
  e o `CAD_RE`.
- Acrescente casos em `tests/test_api.py::test_cad_revisions`.

> ⚠️ Versões já gravadas continuam com o rótulo antigo, e `next_label` precisa conseguir ler o
> último rótulo existente. Se mudar o formato com dados reais, mantenha o formato antigo
> aceito no `CAD_RE`.

## Adicionar um status novo (ex.: "Em fabricação")

**Servidor:**
1. `versioning.py`: acrescente `"manufacturing"` em `STATUSES`. Se só líderes podem atribuir,
   inclua também em `PRIVILEGED_STATUSES`.

**Cliente:**
2. `ui/globals.slint`:
   - `AppState.status-keys`: acrescente `"manufacturing"` na mesma posição em que vai aparecer.
   - `Labels.status`: `: key == "manufacturing" ? @tr("Em fabricação")`.
3. `ui/common.slint` → `StatusColor.of`: escolha uma cor.
4. `ui/views.slint` → `VersionsPanel`: acrescente `Labels.status("manufacturing", AppState.item.kind)`
   ao `model` do `ComboBox`, **na mesma ordem** de `status-keys`.
5. `src/app.rs` → `status_index`: inclua a chave na lista, na mesma ordem.

## Permitir que membros aprovem (sem precisar de admin)

- Servidor: `versioning.py` → `PRIVILEGED_STATUSES = {"released"}` (só a liberação continua
  restrita).
- Cliente: em `ui/views.slint` → `ReviewsView`, troque `AppState.is-admin` por
  `AppState.can-write` no botão "Aprovar".

## Adicionar um campo ao item (ex.: "Material")

**Servidor:**
1. `models.py` → `Item`: `material: Mapped[str | None] = mapped_column(String(100))`.
2. Gere a migração (ver [servidor.md](servidor.md#migrações-alembic)) e revise o arquivo.
3. `schemas.py` → `ItemOut`: `material: str | None`; em `item_out(...)`: `material=item.material`.
4. `services.py` → `create_item`: novo parâmetro `material`, gravado no `Item(...)`.
5. `routers/projects.py` → `create_item`: `material: str | None = Form(None)`, repassado ao serviço.
6. Se for editável depois: crie `PATCH /api/items/{id}` em `routers/items.py`.
7. Teste em `tests/test_api.py`.

**Cliente:**
8. `src/api.rs` → `Item`: `pub material: Option<String>`; em `create_item`, `.text("material", …)`.
9. `ui/globals.slint` → `ItemDetail`: `material: string`.
10. `src/app.rs` → `apply_item`: `material: s(item.material.as_deref().unwrap_or(""))`.
11. `ui/dialogs.slint` → `NewItemDialog`: um `LineEdit` e mais um argumento em `Actions.create-item`.
12. `ui/globals.slint` → assinatura de `create-item`; `src/app.rs` → `on!(on_create_item, …)` e
    `fn create_item`.
13. Mostre o campo em `ui/views.slint` → `ItemPanel` e, se quiser, como coluna em `ItemsTab`
    (lembre de acrescentar o valor em `item_row` no Rust, na mesma posição da coluna).

## Adicionar uma coluna à tabela de itens

1. `ui/views.slint` → `ItemsTab` → `columns`: `{ title: @tr("Responsável"), min-width: 120px }`.
2. `src/app.rs` → `item_row`: devolva o valor na **mesma posição** do vetor.

## Ordenar a tabela ao clicar no cabeçalho (ainda não implementado)

O `StandardTableView` emite `sort-ascending(coluna)` e `sort-descending(coluna)`, mas a
ordenação é responsabilidade do app. Um caminho simples:

1. Em `ItemsTab`, encaminhe os eventos: `sort-ascending(c) => { Actions.sort-items(c, true); }`.
2. Em `app.rs`, ordene `st.items` e `st.locals` juntos pelo campo da coluna e chame
   `apply_items` de novo (os índices de linha continuam alinhados com `st.items`).

## Mudar quem vê o quê no app

As condições de visibilidade ficam nos `.slint`, usando `AppState.is-admin` e
`AppState.can-write`. **O servidor sempre valida de novo**: esconder um botão é só uma questão
de interface, nunca de segurança. As regras reais ficam em `server/paddock_server/deps.py`
(`writer`, `admin`) e `services.py`.

## Traduzir o app

Ver [cliente.md → Tradução](cliente.md#tradução). As mensagens de erro do servidor ficam nos
`raise Invalid(...)`, `Forbidden(...)` etc. em `services.py` e `routers/`.

## Usar PostgreSQL

Ver [servidor.md → Banco PostgreSQL](servidor.md#banco-postgresql-opcional).
