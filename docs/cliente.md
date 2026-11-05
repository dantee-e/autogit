# Cliente desktop

Rust (edição 2024) + [Slint](https://slint.dev) 1.18. Código em `client/`.

## Executar em desenvolvimento

```bash
cd client
cargo run                 # debug
cargo run --release       # mais rápido; é o que vai para os usuários
cargo test                # testes unitários (workspace, formatação, nomes)
```

Linux: `sudo apt install build-essential pkg-config libfontconfig1-dev libxkbcommon-dev`.

Para apontar para outro servidor ou usuário sem mexer na sua configuração, rode com outro
`HOME` (Linux/macOS): `HOME=/tmp/teste cargo run`.

## Estrutura

```
client/
├── build.rs            compila ui/app.slint (estilo "fluent") e, no Windows, embute ícone/versão no .exe
├── src/
│   ├── main.rs         cria a janela, o controlador e o timer de 60 s
│   ├── app.rs          controlador (≈1400 linhas): todas as ações do usuário
│   ├── api.rs          cliente HTTP bloqueante (reqwest) e structs da API
│   ├── workspace.rs    pasta de trabalho (ver pasta-de-trabalho.md)
│   ├── config.rs       config.json do usuário, pasta padrão, nome do computador
│   ├── dialogs.rs      diálogos de arquivo nativos
│   └── fmt.rs          datas (fuso local), tamanhos, "Rev B" / "v1.2.0"
├── ui/
│   ├── app.slint       AppWindow: barra superior, barra lateral, área principal, barra de status, diálogos
│   ├── globals.slint   structs, AppState, Actions, Labels, Theme
│   ├── views.slint     LoginView, Sidebar, ProjectView (Itens/Congelamentos), ReviewsView, ActivityView, AdminView
│   ├── dialogs.slint   NewProject, EditProject, NewItem, Checkout, Checkin, NewBaseline, NewUser, EditUser, Settings, About, Confirm
│   └── common.slint    Badge, Card, Title, Muted, DialogFrame, StatusColor
├── assets/             ícones (png, ico)
└── packaging/          scripts de pacote por sistema
```

## Como a interface fala com o Rust

O Slint e o Rust conversam por **três globais** declarados em `ui/globals.slint`:

| Global | Direção | Papel |
|---|---|---|
| `AppState` | Rust → UI (e UI → UI) | Tudo que a tela mostra: página atual, listas, item selecionado, textos de status/erro, diálogo aberto. |
| `Actions` | UI → Rust | Um `callback` para cada ação do usuário (`login`, `checkout`, `checkin`, `create-baseline`…). |
| `Labels` | Rust e UI | Funções puras que traduzem chaves (`released`, `member`, `outdated`) para texto em português, via `@tr`. O Rust chama, por exemplo, `ui.global::<Labels>().invoke_status(...)` ao montar as linhas das tabelas. |

Os componentes `.slint` **não guardam estado de negócio**: leem `AppState` e chamam `Actions`.
Abrir e fechar diálogos é a única lógica feita só na UI (`AppState.dialog = "checkin"`).

No Rust, `slint::include_modules!()` (em `main.rs`) gera os tipos `AppWindow`, `AppState`,
`Actions`, `Labels` e as structs (`ItemDetail`, `VersionRow`…). Nomes com hífen viram
`snake_case`: `current-item-row` → `get_current_item_row()` / `set_current_item_row()`.

## O controlador (`app.rs`)

### Estado

```rust
pub struct State {            // atrás de Arc<Mutex<_>>
    config: Config,           // servidor, usuário, token, pasta
    api: Option<Api>,         // None = deslogado
    me: Option<User>,
    workspace: Option<Workspace>,
    projects, project_id,     // lista e seleção
    items, locals,            // itens do projeto + estado local de cada um (alinhados por índice)
    item, local, versions,    // item selecionado
    reviews, baselines, baseline, users,
    pending_confirm,          // ação esperando o "OK" do diálogo de confirmação
}
```

`Ctx` (clonável) junta um `Weak<AppWindow>`, o `Arc<Mutex<State>>` e um contador de operações
em andamento. Cada callback de `Actions` recebe um clone do `Ctx` (macro `on!` em `Ctx::wire`).

### Modelo de threads

**Regra:** rede e disco nunca rodam na thread da interface; modelos Slint (`VecModel`, `ModelRc`)
só são criados e alterados na thread da interface.

```rust
ctx.run(
    "Fazendo check-out…",                 // texto na barra de status; "" = silencioso
    move |c| { /* thread de fundo: API, arquivos */ Ok(valor) },
    |c, ui, valor| { /* thread da UI: atualiza AppState */ },
);
```

`Ctx::run`:
1. incrementa o contador e liga `AppState.busy` (botões desabilitam);
2. roda `work` numa `std::thread`;
3. devolve o resultado para a thread da UI com `slint::invoke_from_event_loop`;
4. chama `done` em caso de sucesso ou `show_error` em caso de erro.

`show_error`: se a API respondeu 401 com o usuário na tela principal, volta para o login
("sessão expirou"); em modo silencioso (atualização automática) não mostra nada; senão, exibe a
mensagem do servidor na barra de status (vermelha, com ✕ para fechar).

> ⚠️ **Mutex não reentrante.** `self.st()` trava o estado. Nunca use dois `self.st()` na mesma
> expressão, nem chame uma função que trava enquanto segura o guard: isso **congela a
> interface** (deadlock). Copie o que precisa para variáveis locais primeiro:
>
> ```rust
> let id = c.st().items[r].id;   // guard liberado no fim da linha
> c.load_item(id, false);
> ```

### Carregamento de dados

| Função | Carrega | Quando |
|---|---|---|
| `resume_session` | meta + usuário (com o token salvo) | ao abrir o app |
| `login` → `enter` | meta, login, abre a pasta de trabalho | botão Entrar |
| `refresh(quiet)` | projetos + revisões, depois o projeto atual | botão Atualizar; a cada 60 s (silencioso) |
| `load_project` | itens, congelamentos, estado local de cada item | ao selecionar projeto |
| `load_item` | item, versões, atividade do item, estado local | ao selecionar item e depois de cada ação |

`apply_*` são as funções que, na thread da UI, convertem os dados em linhas de tabela e
preenchem `AppState`. `apply_item` também atualiza a linha do item na tabela, sem recarregar o
projeto inteiro.

A atualização automática (`auto_refresh`, timer em `main.rs`) só roda se a tela principal está
aberta, nada está em andamento e nenhum diálogo está aberto, para não atrapalhar o usuário.

### Confirmações

Para ações destrutivas, `ctx.confirm(ui, título, texto, botão, ação)` guarda a ação em
`pending_confirm` e abre o `ConfirmDialog`. `Actions.confirm-accepted` executa a ação;
`confirm-cancelled` descarta.

### Ações de arquivo

Detalhadas em [pasta-de-trabalho.md](pasta-de-trabalho.md): `checkout`, `checkin`,
`undo_checkout`, `get_latest`, `sync_project`, `fetch_latest`, `create_item`,
`save_version_as`, `download_baseline`.

## `api.rs`

- `Api` é clonável (o `reqwest::blocking::Client` interno é compartilhado). Cada thread de fundo
  pega um clone com `ctx.api()?`.
- Timeout de conexão de 10 s. Chamadas JSON têm timeout de 30 s; uploads e downloads **não têm
  timeout total** (arquivos CAD grandes).
- Erros HTTP viram `ApiError { status, message }`, com `message` = `detail` do servidor.
  `api::is_unauthorized(&e)` detecta 401.
- `download(version_id, dest)` grava num temporário `.<nome>.paddock-tmp` na mesma pasta,
  calcula o SHA-256 durante a escrita, compara com `X-Paddock-SHA256` e só então substitui o
  destino (`workspace::replace_file`). Download corrompido nunca sobrescreve a cópia local.
- Uploads usam `multipart::Part::reader_with_length` (streaming, sem carregar o arquivo inteiro
  na memória).
- TLS via rustls com certificados embutidos (webpki-roots).

## `config.rs`

`Config { server_url, username, token, workspace, open_after_checkout }`, salvo em JSON (escrita
atômica via `.tmp` + `rename`; permissão `0600` no Linux/macOS).

| Sistema | Local |
|---|---|
| Windows | `%APPDATA%\Paddock\Paddock\config\config.json` |
| macOS | `~/Library/Application Support/br.Paddock.Paddock/config.json` |
| Linux | `~/.config/paddock/config.json` |

`device_name()` monta `Paddock desktop (<computador>, <sistema>)`, enviado no login para
identificar a sessão (`COMPUTERNAME` no Windows, `scutil --get ComputerName` no macOS,
`/etc/hostname` ou `hostname` no Linux).

## `dialogs.rs`

`dialogs::ask(Dialog::PickFile | PickFolder | SaveFile, |caminho| …)`. O callback roda na thread
da UI e só se o usuário escolheu algo.

- **macOS:** o AppKit só permite diálogos na thread principal, então usamos o diálogo assíncrono
  do `rfd` como *future* no loop de eventos do Slint (`slint::spawn_local`).
- **Windows/Linux:** diálogo bloqueante numa thread auxiliar (a janela continua redesenhando).
- **Linux:** o `rfd` usa o *xdg-desktop-portal*. Em ambientes sem portal o diálogo não abre; por
  isso os campos de caminho (novo item, pasta de trabalho) também aceitam texto colado.

## Interface (`ui/`)

### Estilo e tema
- Estilo `fluent` em todos os sistemas (definido em `build.rs`), seguindo o tema claro/escuro do
  sistema via `Palette`.
- Cores da marca e de status em `Theme` (`globals.slint`) e `StatusColor` (`common.slint`).
- O selo `AboutSlint` em `AboutDialog` é **obrigatório** pela licença do Slint. Não remova.

### Armadilhas do Slint que já apareceram

| Sintoma | Causa | Solução usada |
|---|---|---|
| Seleção da tabela "desgruda" do estado | `current-row: AppState.x` é ligação de mão única e quebra quando o usuário clica | Usar `current-row <=> AppState.x` (bidirecional) |
| ComboBox mostrando valor antigo | O ComboBox reescreve o próprio `current-index` e perde a ligação calculada | Índice guardado em `AppState.version-status-index`, definido pelo Rust |
| Cabeçalho de coluna cortado ("Ve…") | `min-width` pequeno demais para o título | Larguras mínimas maiores nas colunas curtas |
| Clicar no cabeçalho mostra seta de ordenação, mas não ordena | `StandardTableView` mostra a seta, mas a ordenação é responsabilidade do app | Ainda não implementado (ver customizacao.md) |

### Tradução
Todos os textos visíveis estão em `@tr("…")`. Para traduzir:

```bash
cargo install slint-tr-extractor
slint-tr-extractor ui/*.slint -o paddock.pot
# traduza com Poedit → translations/en/LC_MESSAGES/paddock-client.po
```

Depois, em `build.rs`, acrescente `.with_bundled_translations("translations")` à
`CompilerConfiguration` e escolha o idioma em tempo de execução com
`slint::select_bundled_translation("en")`. As mensagens que vêm do servidor e algumas mensagens
montadas em `app.rs` (barra de status) estão em português direto no código.

## Receita: adicionar uma nova ação

Exemplo: botão "Copiar caminho" no painel do item.

1. **Declare o callback** em `ui/globals.slint`, dentro de `Actions`:
   ```slint
   callback copy-path();
   ```
2. **Coloque o botão** em `ui/views.slint` (`ItemPanel`):
   ```slint
   Button { text: @tr("Copiar caminho"); clicked => { Actions.copy-path(); } }
   ```
3. **Ligue no Rust** em `Ctx::wire` (`src/app.rs`):
   ```rust
   on!(on_copy_path, |c| c.copy_path());
   ```
4. **Implemente** em `impl Ctx`, usando `self.run(...)` se houver rede ou disco:
   ```rust
   fn copy_path(&self) {
       let Some((_, local)) = self.current() else { return };
       self.with_ui(|ui| Ctx::info(ui, local.path.display().to_string()));
   }
   ```
5. Se a ação precisar de uma rota nova, crie-a primeiro no servidor (`services.py` + `routers/`)
   e o método correspondente em `api.rs`.

## Variáveis de ambiente úteis

| Variável | Efeito |
|---|---|
| `SLINT_BACKEND=winit-software` | Renderização por software (VMs, PCs sem OpenGL, área de trabalho remota) |
| `SLINT_BACKEND=winit-femtovg` | Força OpenGL |
| `RUST_BACKTRACE=1` | Detalhes de um eventual travamento |
