# Arquitetura

## Visão geral

```
┌──────────────────────────────┐        HTTPS/HTTP + JSON         ┌──────────────────────────────┐
│ Cliente desktop (Rust+Slint) │  ─────────────────────────────▶  │ Servidor (FastAPI)           │
│  Windows · Linux · macOS     │   Authorization: Bearer <token>  │                              │
│                              │  ◀─────────────────────────────  │  SQLite (metadados)          │
│  Pasta de trabalho local     │      arquivos (multipart /       │  Blobs por SHA-256 (arquivos)│
│  Documentos/Paddock/...      │       download em streaming)     │                              │
└──────────────────────────────┘                                  └──────────────────────────────┘
          ▲                                                                   ▲
          │ abre/salva os arquivos                                            │ curl / CI
     AutoCAD, VS Code…                                              scripts de build de firmware
```

- **Servidor** (`server/`): única fonte da verdade. Guarda usuários, projetos, itens, versões,
  bloqueios (check-outs), status, congelamentos e o histórico de atividade. Não tem interface
  web além da página de configuração inicial e de downloads do app.
- **Cliente** (`client/`): app nativo. Mostra o estado do servidor, mantém uma cópia local dos
  arquivos (a *pasta de trabalho*) e implementa o fluxo check-out → editar → check-in.
- **Scripts/CI**: usam a mesma API com um token de longa duração (ex.: publicar firmware
  compilado, baixar a última versão liberada para gravar no carro).

## Decisões de projeto

| Decisão | Motivo |
|---|---|
| **Sem diff e sem merge** | Arquivos CAD (DWG, STEP…) são binários e não mesclam. O sistema só guarda versões. |
| **Check-out (bloqueio pessimista)** | Como não há merge, a única forma segura de evitar trabalho perdido é uma pessoa por vez. Pode ser obrigatório por projeto (padrão em CAD) ou opcional (padrão em software). |
| **Versões imutáveis, endereçadas por SHA-256** | Uma versão publicada nunca muda; arquivos iguais são guardados uma vez só; dá para verificar integridade a qualquer momento. |
| **Nome de arquivo estável por item** | O nome da primeira versão vira o nome fixo na pasta de trabalho (`manga.dwg`), para as xrefs do AutoCAD continuarem funcionando entre versões. |
| **Rótulos calculados pelo servidor** | O usuário escolhe o *tipo* de mudança (nova revisão, iteração, patch, minor, major) e o servidor gera o rótulo, evitando rótulos duplicados ou fora de ordem. |
| **SQLite por padrão, via SQLAlchemy** | Zero configuração e backup trivial (copiar uma pasta). Dá para migrar para PostgreSQL só trocando a URL. |
| **Arquivos somente leitura sem check-out** | Força o hábito de bloquear antes de editar. Funciona com o atributo nativo de cada sistema. |
| **Cliente nativo (Rust + Slint)** | Um executável pequeno, sem runtime, igual nos três sistemas. Slint sob a licença Royalty-free (ver README). |
| **Mensagens em PT-BR geradas no servidor** | Uma regra de negócio, uma mensagem; o cliente não precisa traduzir códigos de erro. |

## Componentes e arquivos principais

```
server/paddock_server/
├── main.py           cria o app FastAPI, registra rotas e tratadores de erro
├── config.py         configuração por variáveis de ambiente
├── db.py             engine SQLAlchemy, pragmas do SQLite, Alembic
├── models.py         tabelas (ORM)
├── schemas.py        modelos de entrada/saída (Pydantic) e serializadores
├── services.py       REGRAS DE NEGÓCIO (publicar, bloquear, status, congelar)
├── versioning.py     cálculo de rótulos (letras CAD, semver)
├── storage.py        armazenamento de blobs por SHA-256
├── security.py       hash de senhas (scrypt), tokens, limitador de login
├── deps.py           dependências FastAPI: sessão, usuário atual, permissões
├── errors.py         exceções de domínio → códigos HTTP
├── web.py            páginas HTML mínimas (setup, downloads)
├── routers/          rotas da API agrupadas por área
└── migrations/       Alembic

client/
├── src/main.rs       ponto de entrada, timer de atualização automática
├── src/app.rs        CONTROLADOR: liga ações da interface à API e à pasta de trabalho
├── src/api.rs        cliente HTTP e tipos da API
├── src/workspace.rs  pasta de trabalho: estado local, hashes, permissões, backups
├── src/config.rs     configuração do usuário (servidor, token, pasta)
├── src/dialogs.rs    diálogos nativos de arquivo (cada SO do seu jeito)
├── src/fmt.rs        formatação de datas, tamanhos, rótulos
├── ui/app.slint      janela principal
├── ui/globals.slint  estado global, ações, rótulos traduzíveis, tema
├── ui/views.slint    telas (login, projeto, revisões, atividade, administração)
├── ui/dialogs.slint  diálogos (novo projeto, check-in, congelamento…)
└── ui/common.slint   componentes visuais reutilizáveis
```

## Ciclo de vida de um arquivo

```
            Novo item (1ª versão: Rev A / 0.1.0)
                         │
                         ▼
   ┌──────────────▶  [Livre]  ◀─────────────────────────────┐
   │                     │ check-out                         │
   │                     ▼                                   │
   │            [Em check-out com X] ── desfazer check-out ──┤
   │                     │ publicar (check-in)               │
   │                     ▼                                   │
   │          nova versão (Rev B / 0.2.0)                    │
   │          status: Rascunho ─┐                            │
   └────────────────────────────┘                            │
                                                             │
   Status de cada versão (independente do bloqueio):         │
   Rascunho → Em revisão → Aprovado → Liberado → Obsoleto    │
   (aprovar/liberar: só administrador)                       │
                                                             │
   Admin pode "forçar liberação" de qualquer check-out ──────┘
```

Um **congelamento** pode ser criado a qualquer momento e registra, para cada item do projeto, a
última versão (ou a última *liberada*). Versões publicadas depois não alteram o congelamento.
