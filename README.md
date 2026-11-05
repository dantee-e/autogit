# Paddock: gestão de versões para equipes de Formula SAE

Guarda **todas as versões** dos arquivos da equipe: CAD (DWG, DXF, STEP…) e software
(firmware, telemetria). Só versionamento: sem diff e sem merge. Para duas pessoas não
editarem o mesmo arquivo, existe o **check-out** (bloqueio).

```
paddock/
├── server/   API em Python (FastAPI + SQLAlchemy + SQLite)
├── client/   App desktop nativo em Rust + Slint (Windows, Linux, macOS)
└── docs/     Documentação técnica
```

**Documentação técnica:** [arquitetura](docs/arquitetura.md) · [servidor](docs/servidor.md) ·
[API](docs/api.md) · [cliente](docs/cliente.md) · [pasta de trabalho](docs/pasta-de-trabalho.md) ·
[empacotamento](docs/empacotamento.md) · [customização](docs/customizacao.md) ·
[solução de problemas](docs/solucao-de-problemas.md)

## Conceitos

| Conceito | O que é |
|---|---|
| **Projeto** | Agrupa itens de um subsistema. Tipo **CAD** ou **Software**. |
| **Item** | Um arquivo versionado (ex.: “Manga de eixo dianteira”, nº de peça SUS-0142). |
| **Versão** | Cada envio vira uma versão imutável, com comentário, autor, data e SHA-256. |
| **Check-out** | Só quem fez check-out publica no item até liberar. Pode ser obrigatório por projeto (padrão em CAD). |
| **Status** | Rascunho → Em revisão → Aprovado → Liberado → Obsoleto. Só administradores aprovam e liberam. |
| **Congelamento** | Registra qual versão de cada item compõe uma configuração (ex.: “FSAE Brasil 2026”). |

**Rótulos:** CAD usa revisões por letra `A, B, C…` (sem `I O Q S X Z`, como na ASME Y14.35) e
iterações `A.1, A.2`. Software usa versão semântica `MAJOR.MINOR.PATCH`. O sistema calcula
o próximo rótulo; o usuário só escolhe o tipo de mudança.

**Papéis:** *Administrador* (líder: aprova, libera, força liberação de check-out, congela, gerencia
usuários), *Membro* (cria projetos/itens, faz check-out e publica), *Leitor* (só visualiza e baixa).

## Como o app usa a pasta de trabalho

Cada usuário tem uma pasta local (padrão: `Documentos/Paddock`), com uma subpasta por projeto:

```
Documentos/Paddock/
├── Suspensão FS-26/
│   ├── manga.dwg          ← sempre o mesmo nome em todas as versões (xrefs continuam funcionando)
│   └── bandeja_sup.dwg
└── .paddock/              ← estado local e backups (não mexa)
```

- **Check-out**: bloqueia no servidor, baixa a última versão e libera o arquivo para escrita.
- **Publicar (check-in)**: envia o arquivo local como nova versão e devolve o arquivo para somente leitura.
- **Desfazer check-out**: libera o bloqueio. Se havia alterações, elas vão para `.paddock/backup/`
  antes de o arquivo voltar à versão do servidor. **Nada é apagado.**
- Em projetos com check-out obrigatório, arquivos que você não bloqueou ficam **somente leitura**
  (no Windows o AutoCAD abre como “somente leitura”).
- O app compara o arquivo local com o servidor e mostra: *Atualizado*, *Desatualizado*,
  *Alterado localmente* ou *Conflito* (alterado e o servidor tem versão mais nova). Em conflito,
  ele se recusa a publicar, porque não há mesclagem.
- A cada 60 s o app atualiza sozinho os check-outs e as versões novas da equipe.

## Servidor

Requer Python 3.11+.

```bash
cd server
python -m venv .venv
.venv/bin/pip install -e .
.venv/bin/paddock-server serve            # http://0.0.0.0:8000
```

Na primeira vez, abra `http://<servidor>:8000` no navegador para criar o administrador
(ou use `paddock-server create-admin <usuario>` no terminal). A documentação interativa da API
fica em `/docs`. Para os colegas baixarem o app, coloque os instaladores em `data/downloads/`;
eles aparecem na página inicial do servidor.

Docker:

```bash
docker build -t paddock-server server
docker run -d -p 8000:8000 -v paddock-dados:/data --name paddock paddock-server
```

| Variável | Padrão | |
|---|---|---|
| `PADDOCK_DATA` | `./data` | Banco SQLite + arquivos. **Faça backup desta pasta.** |
| `PADDOCK_DATABASE_URL` | SQLite em `PADDOCK_DATA` | Ex.: `postgresql+psycopg://…` (instale o driver) |
| `PADDOCK_TEAM_NAME` | `Equipe Formula SAE` | Nome exibido no app |
| `PADDOCK_SUBSYSTEMS` | Chassi, Suspensão, … | Lista separada por vírgulas |
| `PADDOCK_MAX_UPLOAD_MB` | `1024` | Tamanho máximo por arquivo |
| `PADDOCK_TOKEN_DAYS` | `90` | Validade das sessões do app |
| `PADDOCK_HOST` / `PADDOCK_PORT` | `0.0.0.0` / `8000` | |

Fora da rede da oficina, coloque o servidor atrás de HTTPS (Caddy ou nginx).

**Migrações:** o schema é versionado com Alembic e aplicado sozinho a cada start. Depois de
alterar `paddock_server/models.py`:

```bash
cd server && .venv/bin/python -c "from alembic import command; from paddock_server.db import alembic_config; command.revision(alembic_config('sqlite:///data/paddock.sqlite3'), message='minha mudança', autogenerate=True)"
```

**Testes:** `cd server && .venv/bin/pip install -e '.[dev]' && .venv/bin/pytest`

### Token para scripts e CI

Para o CI publicar firmware compilado, crie um token sem validade:

```bash
curl -X POST -H "Authorization: Bearer <token-de-login>" -H "Content-Type: application/json" \
     -d '{"name": "GitHub Actions"}' http://servidor:8000/api/auth/tokens
```

```bash
curl -H "Authorization: Bearer $PADDOCK_TOKEN" -F file=@build/ecu.hex -F bump=patch \
     -F comment="Build $GITHUB_SHA" http://servidor:8000/api/items/4/versions
```

```bash
curl -OJ -H "Authorization: Bearer $PADDOCK_TOKEN" \
     "http://servidor:8000/api/items/4/versions/latest/download?status=released"
```

## Cliente (app desktop)

Requer Rust estável (https://rustup.rs).

```bash
cd client
cargo run --release
```

Dependências de sistema para **compilar** no Linux (Debian/Ubuntu):
`sudo apt install build-essential pkg-config libfontconfig1-dev libxkbcommon-dev`.
No Windows e no macOS não precisa de mais nada além do Rust (no Mac: `xcode-select --install`).

### Pacotes

| Plataforma | Comando (dentro de `client/`) | Resultado em `client/dist/` |
|---|---|---|
| Linux | `packaging/linux/build-linux.sh` | `.tar.gz` portátil, `.deb` (precisa `cargo install cargo-deb`) e `.AppImage` (precisa do `appimagetool` no PATH) |
| Windows (no Windows) | `powershell packaging\windows\build-windows.ps1` | `.zip` com `Paddock.exe` portátil |
| Windows (a partir do Linux) | `packaging/windows/build-windows-cross.sh` | idem, com mingw |
| macOS (no Mac) | `packaging/macos/build-macos.sh` | `Paddock.app` universal (Apple Silicon + Intel) e `.dmg` |

**macOS sem conta Apple Developer:** o app recebe só assinatura ad-hoc. Na primeira abertura,
cada usuário precisa clicar com o botão direito → **Abrir** (ou rodar
`xattr -dr com.apple.quarantine /Applications/Paddock.app`). O macOS 15+ pede permissão de
“rede local” na primeira conexão com um servidor da oficina; aceite.

Se o app não abrir numa máquina virtual ou num PC sem aceleração gráfica, force o renderizador
por software: `SLINT_BACKEND=winit-software`.

### Onde o app guarda as coisas

| | Configuração e sessão | Pasta de trabalho padrão |
|---|---|---|
| Windows | `%APPDATA%\Paddock\Paddock\config\config.json` | `Documentos\Paddock` |
| macOS | `~/Library/Application Support/br.Paddock.Paddock/config.json` | `~/Documents/Paddock` |
| Linux | `~/.config/paddock/config.json` (permissão 600) | `~/Documentos/Paddock` |

## Onde customizar

| Quero mudar… | Arquivo |
|---|---|
| Regras de numeração (letras, semver) | `server/paddock_server/versioning.py` |
| Quem pode fazer o quê | `server/paddock_server/services.py` e `deps.py` |
| Subsistemas padrão | `server/paddock_server/config.py` (ou `PADDOCK_SUBSYSTEMS`) |
| Mensagens de erro do servidor (PT-BR) | `server/paddock_server/services.py`, `routers/*.py` |
| Telas do app | `client/ui/views.slint`, `client/ui/dialogs.slint` |
| Cores, nomes de status e papéis | `client/ui/globals.slint` (`Theme`, `Labels`) |
| Fluxo check-out/check-in, sincronização | `client/src/app.rs` |
| Regras da pasta de trabalho (somente leitura, backups) | `client/src/workspace.rs` |
| Ícone | `client/assets/` (`icon-1024.png`, `icon.ico`) |

**Tradução:** todos os textos das telas usam `@tr("…")`. Para gerar um `.pot` e traduzir
(por exemplo, para vender a equipes de fora):
`slint-tr-extractor ui/*.slint -o paddock.pot` (`cargo install slint-tr-extractor`), e depois
carregar as traduções com `slint_build::CompilerConfiguration::with_bundled_translations`.

## Licenças

- O app usa o **Slint** sob a *Slint Royalty-free Desktop, Mobile, and Web Applications License 2.0*:
  uso e venda gratuitos de apps desktop proprietários, **desde que** o widget `AboutSlint` apareça
  na tela “Sobre” acessível pelo menu principal (já está em `client/ui/dialogs.slint`; não remova)
  e o app não exponha a API do Slint como biblioteca. Uso em sistemas embarcados exige licença paga.
  Texto completo em `~/.cargo/registry/src/*/slint-1.*/LICENSES/`.
- As demais dependências (FastAPI, SQLAlchemy, reqwest, rfd etc.) são MIT/Apache/BSD.
