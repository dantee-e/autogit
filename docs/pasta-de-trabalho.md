# Pasta de trabalho

Implementação em `client/src/workspace.rs` (estado e operações de arquivo) e nas ações de
`client/src/app.rs` (`checkout`, `checkin`, `undo_checkout`, `get_latest`, `sync_project`,
`fetch_latest`, `create_item`).

## Layout

```
<raiz>/                                  padrão: Documentos/Paddock
├── <Nome do projeto>/                   nome limpo para Windows/Linux/macOS
│   └── <nome fixo do item>              ex.: manga.dwg (= file_name vindo da API)
└── .paddock/                            oculta (atributo +h no Windows; ponto no Unix)
    ├── <servidor>.json                  estado local, ex.: 192.168.0.10_8000.json
    └── backup/                          cópias de segurança: manga_20261105-135140.dwg
```

- Cada servidor tem seu próprio arquivo de estado, então a mesma pasta pode ser usada com dois
  servidores sem confundir os IDs dos itens.
- Nomes são passados por `workspace::sanitize`: troca `<>:"/\|?*` e caracteres de controle por
  `_`, remove pontos e espaços finais e prefixa `_` em nomes reservados do Windows (`CON`,
  `PRN`, `AUX`, `NUL`, `COM1-9`, `LPT1-9`).

## Estado local de cada item

O arquivo `.paddock/<servidor>.json` guarda, por ID de item:

```json
{
  "path": "Suspensão FS-26/manga.dwg",
  "version_id": 7,          // versão do servidor em que a cópia se baseia
  "label": "C",
  "base_sha256": "6978…",   // hash do conteúdo baixado/publicado
  "cached_mtime": 1762350700123456789,
  "cached_size": 70012,
  "cached_sha256": "6978…"  // hash atual do arquivo, válido enquanto mtime/size não mudam
}
```

O cache de hash evita reler arquivos CAD grandes a cada atualização: o arquivo só é lido de
novo quando a data de modificação ou o tamanho mudam.

## Como o estado é calculado (`Workspace::inspect`)

```
arquivo não existe ─────────────────────────────────────────▶ Missing     "Não baixado"
existe, mas sem registro no estado:
    hash == última versão do servidor ──▶ registra (adota) ─▶ UpToDate    "Atualizado"
    hash diferente ─────────────────────────────────────────▶ Untracked   "Arquivo local desconhecido"
existe e registrado:
    modificado = hash atual ≠ base_sha256
    desatualizado = version_id ≠ última versão do servidor
        não modificado, atualizado ─────────────────────────▶ UpToDate    "Atualizado"
        não modificado, desatualizado ──────────────────────▶ Outdated    "Desatualizado"
        modificado, atualizado ─────────────────────────────▶ Modified    "Alterado localmente"
        modificado, desatualizado ──────────────────────────▶ Conflict    "Conflito"
```

`is_modified()` é verdadeiro para `Modified`, `Conflict` e `Untracked`: são os casos em que
há trabalho local que o servidor não tem.

## Permissões (somente leitura)

`workspace::set_writable(caminho, bool)`:

| Sistema | Somente leitura | Editável |
|---|---|---|
| Windows | atributo `FILE_ATTRIBUTE_READONLY` (o AutoCAD abre como "somente leitura") | remove o atributo |
| Linux / macOS | remove os bits de escrita (`chmod a-w`) | adiciona só a escrita do dono (`u+w`) |

> No Unix não usamos `Permissions::set_readonly(false)` da biblioteca padrão: ele dá escrita
> para **todos** os usuários.

Regra aplicada depois de toda operação:

```
editável = projeto NÃO exige check-out  OU  o item está em check-out com você
```

## Substituição segura de arquivos

`workspace::replace_file(tmp, destino)` (usado por todos os downloads):

1. tira o somente leitura do destino, porque o Windows não deixa substituir arquivo somente leitura;
2. `rename` atômico do temporário para o destino (na mesma pasta, então nunca fica meio arquivo);
3. no Windows, erro 32 (arquivo em uso) vira a mensagem "O arquivo … está aberto em outro
   programa. Feche-o e tente novamente."

## Ações

### Fazer check-out
1. `POST /api/items/{id}/lock` (falha se alguém já bloqueou).
2. Decide o que fazer com a cópia local:

   | Estado local | O que acontece |
   |---|---|
   | Missing / Outdated | baixa a última versão |
   | UpToDate | nada |
   | Modified (baseado na última) | **mantém** o arquivo: o usuário continua o próprio trabalho |
   | Conflict / Untracked | copia o arquivo para `.paddock/backup/` e baixa a última |

3. Deixa o arquivo editável e, se a opção estiver ligada, abre no programa padrão.

### Publicar nova versão (check-in)
Só habilitado se a cópia local tem mudanças. Para cada estado local:

| Estado | Resultado |
|---|---|
| Missing | erro: arquivo não está na pasta |
| UpToDate / Outdated | erro: nada para publicar |
| **Conflict** | **recusado**: a cópia foi feita a partir de uma versão antiga e não há mesclagem. A mensagem orienta a salvar as mudanças com outro nome, baixar a última versão e reaplicá-las. |
| Modified / Untracked | publica |

Depois de publicar: busca o item atualizado, confere se o hash do arquivo ainda é o enviado
(se mudou durante o upload, avisa para fechar o programa e publicar de novo), registra a nova
base e aplica a regra de permissão (o check-out é liberado, a menos que "Manter check-out" esteja
marcado).

### Desfazer check-out
- Se a cópia **não** tem mudanças: só libera o bloqueio.
- Se **tem** mudanças: pede confirmação, libera o bloqueio, copia o arquivo para
  `.paddock/backup/`, baixa a última versão e deixa somente leitura.
- Administrador desbloqueando o item de outra pessoa ("Forçar liberação"): pede confirmação e
  só libera no servidor, sem mexer nos arquivos locais.

### Baixar última
Se há mudanças locais, pede confirmação e faz backup antes. Depois baixa e aplica a regra de
permissão.

### Sincronizar (projeto inteiro)
Para cada item: baixa se `Missing` ou `Outdated`; corrige as permissões se `UpToDate`;
**pula** qualquer item com mudanças locais. Mostra quantos baixou e quantos pulou.

### Novo item
Envia o arquivo escolhido como 1ª versão. Depois **copia** esse arquivo para a pasta de
trabalho com o nome fixo (se ele já não estava lá), registra a base e aplica a regra de permissão.
O arquivo original escolhido não é alterado.

### Salvar como / baixar congelamento
Não tocam na pasta de trabalho: gravam onde o usuário escolher, com nomes no formato
`manga_revB.dwg`. O congelamento cria `<projeto> - <congelamento>/<item>/<arquivo>` e um
`MANIFESTO.txt` com rótulo, status e SHA-256 de cada arquivo.

## Garantias

- **Nada é apagado.** Toda substituição de um arquivo com mudanças passa antes por
  `.paddock/backup/`.
- **Download verificado.** O arquivo só substitui a cópia local depois que o SHA-256 bate com o
  do servidor.
- **Escrita atômica.** O estado e os downloads usam temporário + `rename`.

## Limitações conhecidas

- Se a extensão do arquivo mudar entre versões (`.dwg` → `.dxf`), o `file_name` passa a ter a
  nova extensão e a cópia antiga fica na pasta como arquivo solto.
- Renomear um projeto no servidor (ainda não há rota para isso) mudaria o nome da pasta local.
- A pasta de trabalho é por computador e por usuário do sistema. Duas pessoas usando o mesmo
  login do sistema no mesmo PC compartilham a mesma pasta.
