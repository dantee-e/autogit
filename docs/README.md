# Documentação do Paddock

Documentação técnica para quem vai manter, customizar ou estender o Paddock. Para instalar e
usar, comece pelo [README principal](../README.md).

| Documento | Para quem | Conteúdo |
|---|---|---|
| [Arquitetura](arquitetura.md) | Todos | Visão geral, componentes, decisões de projeto |
| [Servidor](servidor.md) | Quem mexe no backend | Módulos Python, modelo de dados, regras de negócio, migrações, segurança |
| [Referência da API](api.md) | Quem integra (CI, scripts) ou mexe no cliente | Todas as rotas, parâmetros, respostas e erros |
| [Cliente desktop](cliente.md) | Quem mexe no app Rust/Slint | Módulos, modelo de threads, estado, interface, como adicionar uma tela ou ação |
| [Pasta de trabalho](pasta-de-trabalho.md) | Quem mexe no fluxo de arquivos | Estados locais, check-out/check-in, sincronização, backups, permissões |
| [Empacotamento](empacotamento.md) | Quem gera versões para a equipe | Builds e pacotes para Windows, Linux e macOS |
| [Customização](customizacao.md) | Quem adapta para outra equipe | Receitas passo a passo para as mudanças mais comuns |
| [Solução de problemas](solucao-de-problemas.md) | Suporte | Erros comuns e como resolver |

## Convenções do código

- **Idioma:** identificadores, rotas da API e comentários em **inglês**; tudo que o usuário lê
  (interface, mensagens de erro do servidor) em **português**.
- **Textos da interface** sempre dentro de `@tr("…")` nos arquivos `.slint`, para permitir tradução.
- **Mensagens de erro do servidor** são escritas para o usuário final: o cliente as mostra sem
  alteração na barra de status.
- **Chaves internas em inglês** (`draft`, `released`, `member`, `revision`…) trafegam na API; os
  rótulos em português ficam só no cliente (`client/ui/globals.slint`, global `Labels`).
