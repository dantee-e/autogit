# Empacotamento e distribuição

Todos os scripts ficam em `client/packaging/` e gravam em `client/dist/`. A versão vem do
campo `version` de `client/Cargo.toml`; mude lá antes de gerar uma versão nova.

## Resumo

| Alvo | Onde rodar | Comando (dentro de `client/`) | Resultado |
|---|---|---|---|
| Linux portátil | Linux | `packaging/linux/build-linux.sh` | `paddock-<v>-linux-x86_64.tar.gz` |
| Linux .deb | Linux + `cargo install cargo-deb` | (mesmo script) | `paddock_<v>-1_amd64.deb` |
| Linux AppImage | Linux + `appimagetool` no PATH (ou `APPIMAGETOOL=/caminho`) | (mesmo script) | `Paddock-<v>-x86_64.AppImage` |
| Windows portátil | Windows | `powershell packaging\windows\build-windows.ps1` | `paddock-<v>-windows-x64.zip` |
| Windows portátil | Linux (cross) | `packaging/windows/build-windows-cross.sh` | idem |
| macOS | Mac | `packaging/macos/build-macos.sh` | `Paddock.app` universal + `Paddock-<v>-macos.dmg` |

Para a equipe baixar, copie os arquivos de `dist/` para `PADDOCK_DATA/downloads/` no servidor:
eles aparecem na página inicial (`http://servidor:8000/`).

## Perfil de release

Em `Cargo.toml`: `strip = true`, `lto = "thin"`, `codegen-units = 1`. O build demora mais (de 5
a 8 minutos do zero), mas o binário fica bem menor (cerca de 18 MB no Windows e 25 MB no Linux,
com o Slint, o renderizador e o TLS embutidos).

## Linux

Dependências de **build** (Debian/Ubuntu): `build-essential pkg-config libfontconfig1-dev libxkbcommon-dev`.

Dependências de **execução**: o `.deb` declara todas (`Depends:` em `[package.metadata.deb]`):
`libxkbcommon-x11-0`, `libfontconfig1`, `libgl1`/`libegl1`, X11 (`libx11-6`, `libxcursor1`,
`libxrandr2`, `libxi6`) e Wayland (`libwayland-client0`, `-cursor0`, `-egl1`). Também recomenda
o `xdg-desktop-portal`, usado pelos diálogos de arquivo. Para o `.tar.gz` e o AppImage, essas
bibliotecas já existem em qualquer desktop comum (GNOME, KDE, XFCE).

O `.deb` instala:
- `/usr/bin/paddock`
- `/usr/share/applications/paddock.desktop` (menu de aplicativos)
- `/usr/share/icons/hicolor/256x256/apps/paddock.png`

## Windows

- Nativo: instale o Rust com o instalador do https://rustup.rs (ele oferece as ferramentas do
  Visual Studio). Rode o `.ps1`.
- Cross a partir do Linux:
  ```bash
  rustup target add x86_64-pc-windows-gnu
  sudo apt install gcc-mingw-w64-x86-64 zip
  packaging/windows/build-windows-cross.sh
  ```
- O `build.rs` embute ícone e informações de versão no `.exe` (crate `winresource`; no cross,
  usa o `windres` do mingw).
- Em builds de release, `windows_subsystem = "windows"` (em `main.rs`) esconde a janela de
  console.
- O pacote é **portátil**: um único `Paddock.exe`, sem instalador. A configuração fica em
  `%APPDATA%`. O Windows SmartScreen pode avisar na primeira execução (o app não é assinado):
  "Mais informações → Executar assim mesmo".

## macOS

Rode no Mac (Apple Silicon ou Intel), com `xcode-select --install` e o Rust instalados:

```bash
packaging/macos/build-macos.sh
```

O script:
1. compila para `aarch64-apple-darwin` e `x86_64-apple-darwin` (mínimo macOS 11);
2. junta os dois num binário universal com `lipo`;
3. monta `Paddock.app` com `packaging/macos/Info.plist` (inclui as permissões de **rede local**
   e de **Documentos**);
4. gera o `.icns` a partir de `assets/icon-1024.png` (`sips` + `iconutil`);
5. assina **ad-hoc** (`codesign --sign -`, grátis e obrigatório para rodar no Apple Silicon);
6. cria o `.dmg` com `hdiutil`.

**Sem conta Apple Developer**, o app não é notarizado. Na primeira abertura, cada usuário
precisa clicar com o botão direito no app → **Abrir** → **Abrir**, ou rodar:

```bash
xattr -dr com.apple.quarantine /Applications/Paddock.app
```

Se um dia quiserem distribuir sem esse passo, é preciso uma conta Apple Developer para assinar
com "Developer ID" e notarizar (`xcrun notarytool`). O script já deixa o caminho pronto: basta
trocar o `--sign -` pela identidade de assinatura.

## Servidor

- Instalação direta: `pip install ./server` cria o comando `paddock-server`.
- Docker: `docker build -t paddock-server server` (imagem `python:3.13-slim`, dados em `/data`).
- Exemplo de serviço systemd:

  ```ini
  # /etc/systemd/system/paddock.service
  [Unit]
  Description=Paddock server
  After=network.target

  [Service]
  User=paddock
  Environment=PADDOCK_DATA=/srv/paddock
  Environment=PADDOCK_TEAM_NAME=Equipe Tupã Racing
  ExecStart=/opt/paddock/.venv/bin/paddock-server serve --port 8000
  Restart=on-failure

  [Install]
  WantedBy=multi-user.target
  ```

- HTTPS com Caddy (certificado automático):

  ```
  paddock.equipe.com.br {
      reverse_proxy 127.0.0.1:8000
      request_body {
          max_size 1GB
      }
  }
  ```

## Checklist de nova versão

1. Atualize `version` em `client/Cargo.toml` e `server/pyproject.toml` /
   `server/paddock_server/__init__.py`.
2. `cd server && .venv/bin/pytest` e `cd client && cargo test`.
3. Gere os pacotes de cada sistema.
4. Teste ao menos: login, check-out, editar, publicar, desfazer check-out.
5. Copie os pacotes para `PADDOCK_DATA/downloads/` no servidor.
6. Se mudou o modelo de dados, a migração roda sozinha no próximo start. **Faça backup de
   `PADDOCK_DATA` antes de atualizar o servidor.**
