# Solução de problemas

## App

| Sintoma | Causa provável | Solução |
|---|---|---|
| "Não foi possível conectar ao servidor" | Endereço errado, servidor parado ou firewall | Abra `http://<servidor>:8000/api/meta` no navegador. Confira a porta e se o PC está na mesma rede. |
| "Este servidor ainda não tem usuários" | Primeiro uso | Abra `http://<servidor>:8000` no navegador e crie o administrador. |
| "Sua sessão expirou" | Token expirou (padrão: 90 dias), senha redefinida ou usuário desativado | Entre de novo. |
| "Muitas tentativas" | 5 senhas erradas em 5 minutos | Espere 5 minutos (ou reinicie o servidor). |
| Botão "Publicar" desabilitado | A cópia local não tem mudanças | Salve o arquivo no AutoCAD/editor e clique no item para atualizar. |
| "Este projeto exige check-out" | Projeto com check-out obrigatório | Faça check-out antes de editar e publicar. |
| "Em check-out com Fulano" | Outra pessoa bloqueou | Peça para ela publicar ou desfazer o check-out; em último caso, um admin usa "Forçar liberação". |
| "Sua cópia foi feita a partir da Rev X, mas o servidor já tem a Rev Y" | Conflito (só acontece em projetos sem check-out obrigatório) | Salve seu arquivo com outro nome, clique em "Baixar última", reaplique as mudanças e publique. |
| "O arquivo … está aberto em outro programa" (Windows) | AutoCAD/Excel com o arquivo aberto | Feche o arquivo e repita. |
| "O arquivo mudou durante o envio" | O programa salvou de novo enquanto enviava | Feche o arquivo e publique de novo. |
| "O arquivo é idêntico à versão atual" | Nada mudou de fato | Nenhuma ação necessária. |
| "Já existe um item com o arquivo X neste projeto" | Dois itens com o mesmo nome de arquivo | Renomeie o arquivo antes de criar o item. |
| AutoCAD abre o arquivo como "somente leitura" | Você não fez check-out (esperado) | Faça check-out no Paddock. |
| Diálogo de arquivo não abre (Linux) | Sem `xdg-desktop-portal` | `sudo apt install xdg-desktop-portal-gtk` (ou `-kde`), ou cole o caminho no campo. |
| Janela não abre / tela preta (VM, área de trabalho remota) | Sem OpenGL | Rode com `SLINT_BACKEND=winit-software`. |
| Linux: o app fecha na hora citando `xkbcommon` | Falta `libxkbcommon-x11` | `sudo apt install libxkbcommon-x11-0` (o `.deb` já instala). |
| macOS: "não pode ser aberto porque o desenvolvedor não pode ser verificado" | App sem notarização | Botão direito → Abrir, ou `xattr -dr com.apple.quarantine /Applications/Paddock.app`. |
| macOS: não conecta ao servidor da oficina | Permissão de rede local negada | Ajustes do Sistema → Privacidade e Segurança → Rede Local → ative o Paddock. |
| Windows: SmartScreen bloqueia | Executável não assinado | "Mais informações" → "Executar assim mesmo". |

### Onde estão minhas alterações descartadas?
Em `<pasta de trabalho>/.paddock/backup/`, com data e hora no nome
(`manga_20261105-135140.dwg`). No Windows a pasta `.paddock` é oculta: ative "Itens ocultos" no
Explorador.

### Recomeçar a pasta de trabalho do zero
Feche o app, renomeie a pasta de trabalho (por segurança) e abra o app de novo; depois use
**Sincronizar** em cada projeto. Itens em check-out com você continuam bloqueados no servidor.

### Esquecer o login salvo
Apague o `config.json` (caminhos em [cliente.md](cliente.md#configrs)) ou clique em **Sair**.

## Servidor

| Sintoma | Solução |
|---|---|
| Upload de arquivo grande falha | Aumente `PADDOCK_MAX_UPLOAD_MB` **e** o limite do proxy (`client_max_body_size` no nginx, `request_body max_size` no Caddy). |
| `database is locked` | Duas instâncias do servidor usando o mesmo `PADDOCK_DATA`. Rode só uma. |
| Erro de migração ao iniciar | Restaure o backup de `PADDOCK_DATA` e verifique o arquivo de migração novo. |
| Esqueci a senha do único admin | No servidor: `PADDOCK_DATA=… paddock-server create-admin novo_admin`, entre com ele e redefina a senha antiga em Administração. |
| Conferir se algum arquivo corrompeu | `GET /api/items/{id}/verify` para cada item, ou compare os SHA-256 do `MANIFESTO.txt` de um congelamento. |

## Coletando informações para reportar um bug

1. Versão do app (Sobre) e do servidor (`/api/meta`).
2. Sistema operacional.
3. Mensagem exata da barra de status.
4. Para travamentos: rode o app pelo terminal com `RUST_BACKTRACE=1` e copie a saída.
5. Log do servidor (saída do `paddock-server serve`), na hora do erro.
