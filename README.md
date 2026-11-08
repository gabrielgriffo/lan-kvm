<div align="center">
  <h1>
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="branding/lan-kvm-horizontal-color-on-dark.svg">
      <img src="branding/lan-kvm-horizontal-color.svg" width="360" alt="Lan Kvm" />
    </picture>
  </h1>

  <p>
    Um teclado e um mouse para um PC com Windows e um notebook com Linux na mesma rede, com o monitor trocando de entrada junto.<br>
  </p>

[![Release](https://img.shields.io/github/v/release/gabrielgriffo/lan-kvm?color=blue)](https://github.com/gabrielgriffo/lan-kvm/releases/latest) [![License](https://img.shields.io/github/license/gabrielgriffo/lan-kvm?label=license&color=yellow)](LICENSE) [![Built with Rust](https://img.shields.io/badge/built_with-Rust-orange)](https://www.rust-lang.org/)

  <p>
    <a href="https://github.com/gabrielgriffo/lan-kvm/releases/latest">
      <img src="https://img.shields.io/badge/Download-Windows_·_Linux-6b7280?style=for-the-badge&logo=github&logoColor=white&labelColor=16a34a" alt="Baixar o Lan Kvm" /></a>
  </p>

</div>

Empurre o mouse para fora da borda da tela e ele continua no notebook, como num segundo monitor. Ou
bata duas vezes no topo da tela e o monitor troca para o notebook levando o mouse e o teclado junto.
O clipboard vai com você, e nada sai da sua rede.

## Início rápido

1. **Instale no notebook.**

   ```sh
   sudo apt install ./lan-kvm_<versão>_amd64.deb
   ```

   Saia da sessão e entre de novo. O receptor inicia sozinho com a sessão gráfica e escuta na porta
   TCP 24801. Se o firewall do notebook estiver ativo, libere essa porta para o PC.

2. **Instale no PC.** Rode o `lan-kvm-<versão>-windows-setup.exe`. Se o Windows mostrar "O Windows
   protegeu o computador", escolha Mais informações e depois Executar assim mesmo.

3. **Pareie.** Na janela do Lan Kvm, digite o IP ou o nome do notebook, escolha a posição dele e
   clique em Salvar. Depois clique em Parear com o notebook: aparece um código de 6 dígitos numa
   notificação do notebook. Digite o código no PC. O pareamento é feito uma vez só.

4. **Atravesse.** Empurre o mouse pela borda do lado em que o notebook está. Para voltar, empurre
   pela borda oposta do notebook. Ctrl+Alt+Backspace devolve o controle ao Windows a qualquer momento.

## Interface

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: light)" srcset="assets/janela-claro.png">
    <img src="assets/janela-escuro.png" width="420" alt="Janela do Lan Kvm conectada ao notebook: endereço, posição, opções de troca de monitor por duas batidas no topo e o status da conexão no rodapé">
  </picture>
</p>

## O que faz

- **Atravessa pela borda.** O mouse sai pela borda do PC e entra no notebook na mesma altura
  proporcional. A borda pode ficar à esquerda, à direita ou desativada, e os 5% de cima e de baixo
  podem ficar protegidos para não trocar de tela sem querer.
- **Troca o monitor junto.** Com um monitor ligado nas duas máquinas, o Lan Kvm troca a entrada dele
  por DDC/CI no mesmo momento em que o controle passa: o PC pede a entrada do notebook e o notebook
  pede a do PC na volta.
- **Duas batidas no topo.** Bata o mouse duas vezes na borda de cima, em menos de 1 s, e o monitor
  alterna entre o PC e o notebook, com o mouse e o teclado indo para quem aparecer na tela. Cada
  lado lembra onde o mouse estava e ele volta para lá.
- **Funciona nos dois sentidos.** O touchpad e o teclado do próprio notebook também atravessam para o
  PC pela borda voltada para ele.
- **O mouse tem a mesma sensação.** O notebook reproduz a velocidade e a aceleração do mouse
  configuradas no Windows, então o ponteiro anda igual nas duas telas.
- **Leva o clipboard.** O texto copiado vai junto a cada troca de controle, nos dois sentidos.
- **Leva o som e o microfone.** Opcional: o som do notebook toca no fone do PC e o microfone do PC
  vira o microfone do notebook enquanto estiverem conectados.
- **Devolve o monitor ao desligar.** Se o notebook for desligado, se você sair da sessão nele ou se a
  conexão cair com o monitor no notebook, a entrada volta para o PC.
- **Fica na bandeja.** Fechar a janela mantém o Lan Kvm rodando na bandeja, e ele pode iniciar com o
  Windows. Abrir o app de novo só traz a janela que já está aberta para a frente.
- **Só na sua rede.** O PC fala direto com o notebook, cifrado, depois de um pareamento com código.
  Não há conta, servidor no meio, telemetria nem verificação de atualizações. Mais em
  [SECURITY.md](SECURITY.md).

## Configurações

Tudo fica numa janela só, no PC. O notebook não tem configuração própria.

| Opção | O que faz |
|---|---|
| Notebook | IP ou nome do notebook na rede. |
| Posição | Em que lado do PC o notebook fica: à esquerda, à direita ou desativado (só as duas batidas trocam). |
| Esquecer pareamento | Apaga o segredo do pareamento neste PC. |
| Ativado | Liga e desliga o Lan Kvm sem fechar o app. |
| Proteger os cantos da borda | Os 5% de cima e de baixo da borda não trocam de tela. |
| Trocar a entrada do monitor junto com o controle | Escolhe o monitor e as entradas do PC e do notebook. |
| Trocar a entrada do monitor com duas batidas no topo | Liga as duas batidas, com o mesmo monitor e as mesmas entradas. |
| Usar o fone e o microfone do PC no notebook | Leva o som e o microfone, mostrando o dispositivo usado em cada lado. |
| Iniciar com o Windows | Abre o Lan Kvm na bandeja ao entrar no Windows. |

O status da conexão fica no rodapé da janela e na cor do ícone da bandeja: cinza desativado,
laranja esperando o notebook, verde conectado e azul-turquesa com o controle no notebook.

## Limites conhecidos

- **Só um PC com Windows e um notebook com Linux.** Não há suporte a macOS, a duas máquinas Windows
  ou a mais de um notebook.
- **Requisitos.** PC com Windows 10 ou 11, 64 bits. Notebook com Ubuntu 22.04 ou mais novo (glibc
  2.35+), 64 bits. Os dois precisam estar na mesma versão do Lan Kvm.
- **Só X11 no notebook.** No Wayland o receptor não consegue injetar nem capturar o mouse e o teclado.
- **Arquivos não atravessam**, só texto no clipboard, até 1 MiB.
- **A troca de monitor depende do monitor.** Ele precisa aceitar DDC/CI, e alguns numeram as entradas
  fora do padrão: vale o código que funcionar.
- **O instalador do Windows não é assinado.** O SmartScreen e alguns antivírus podem perguntar na
  primeira vez.

## Em detalhe

<details>
<summary><strong>Estados da conexão</strong></summary>

| Status | Significado |
|---|---|
| `Desativado` | O Lan Kvm está desligado na janela ou no menu da bandeja. |
| `Conectando a <endereço>…` | O PC está tentando chegar ao notebook. |
| `Sem conexão com <endereço>: <erro>` | O notebook não respondeu. Confira se ele está ligado, na sessão X11 e com a porta 24801 liberada. |
| `Este PC ainda não está pareado com <endereço>` | Falta parear. Clique em Parear com o notebook. |
| `Conectado a <endereço> (tela remota LxA)` | Conexão cifrada no ar, controle no PC. |
| `Controle no notebook (<endereço>)` | O mouse e o teclado do PC estão no notebook. |
| `Notebook controlando o PC (<endereço>)` | O touchpad e o teclado do notebook estão no PC. |

Uma queda de conexão devolve o controle ao PC na hora e, se o monitor estava no notebook, ele volta
para a entrada do PC.

</details>

<details>
<summary><strong>Troca de monitor</strong></summary>

O monitor só aceita comandos DDC/CI da entrada que está mostrando. Por isso o PC troca para a entrada
do notebook e o notebook troca de volta para a do PC. No PC, Detectar lista os monitores que
respondem e as entradas de cada um. No notebook, o pacote `.deb` carrega o módulo `i2c-dev` e libera
o acesso ao barramento para o usuário da sessão, sem precisar de root.

Para não deixar o monitor preso numa máquina sem controle, o notebook devolve a entrada do PC quando
o receptor é encerrado: desligamento, saída da sessão ou queda da conexão.

</details>

<details>
<summary><strong>Duas batidas no topo</strong></summary>

Duas batidas na borda de cima, em menos de 1 s, alternam o monitor escolhido entre a entrada do PC e
a do notebook. Vale para o mouse do PC, esteja ele na tela do PC ou na do notebook (o touchpad do
notebook não conta), e não depende da Posição, então dá para usar
só as batidas com a Posição em Desativado.

Ao chegar, o mouse aparece onde estava da última vez naquela tela. Na primeira vez, ou se aquela
posição ficou fora da tela, ele aparece no centro do monitor.

</details>

<details>
<summary><strong>Mouse, teclado e clipboard</strong></summary>

O PC lê o movimento cru do mouse e aplica a curva de aceleração do Windows antes de enviar, então a
velocidade no notebook é a mesma do PC. Com o controle no notebook, o cursor do Windows fica
escondido por uma janela invisível sob ele, sem mexer nos cursores do sistema.

As teclas viajam pela posição física, então o layout que vale é o do notebook. Ctrl+Alt+Backspace é
sempre do PC: devolve o controle ao Windows e não é enviado.

O clipboard troca de lado no momento em que o controle passa, só quando mudou desde a última troca.
Vai só texto, até 1 MiB.

</details>

<details>
<summary><strong>Som e microfone</strong></summary>

Com a opção ligada, o notebook grava o próprio som e o envia para o PC, que toca no dispositivo de
saída padrão do Windows. Na direção contrária, o microfone padrão do PC aparece como microfone no
notebook. No notebook isso usa o PulseAudio ou o PipeWire com `pactl`, `parec` e `pacat`, que já
vêm no Ubuntu. O áudio vai cifrado pela mesma conexão.

</details>

<details>
<summary><strong>Linha de comando</strong></summary>

```text
lan-kvm                                abre a interface (Windows)
lan-kvm send <host[:porta]> [--left] [--protect-corners] [--monitor NOME,PC,NOTEBOOK [--monitor-now]]
                                       captura mouse e teclado sem interface (Windows)
lan-kvm pair <host[:porta]>            pareia com o notebook (Windows)
lan-kvm recv [endereço:porta]          recebe mouse e teclado (Linux X11)
lan-kvm unpair                         remove o pareamento desta máquina
lan-kvm monitors                       lista monitores e entradas por DDC/CI
lan-kvm --version                      mostra a versão
```

No Windows instalado, o executável é `%LOCALAPPDATA%\Programs\Lan Kvm\Lan Kvm.exe`. Depois de
pareado, o notebook recusa novos pareamentos até rodar `lan-kvm unpair` nele.

</details>

<details>
<summary><strong>Compilar a partir do código</strong></summary>

Pré-requisitos:

- [Rust](https://www.rust-lang.org/tools/install) 1.99 ou mais novo, toolchain `stable`
- No Windows, para o instalador: [Inno Setup 6](https://jrsoftware.org/isinfo.php)
- No Linux: `pkg-config` e os pacotes de desenvolvimento do X11 (`libx11-dev`, `libxi-dev`,
  `libxtst-dev`, `libxcb1-dev`, `libxkbcommon-dev`, `libxkbcommon-x11-dev`), ou Docker

```powershell
# Windows: fmt, clippy, testes e build release
.\scripts\build-windows.ps1

# Windows: instalador em target\installer (com -Install, instala nesta máquina)
.\scripts\package-windows.ps1
```

```sh
# Linux: compila e testa num container Ubuntu 22.04
bash scripts/build-linux-docker.sh

# Linux: gera o .deb a partir do binário
bash scripts/package-deb.sh <binário>
```

Do PC, `.\scripts\build-linux-remote.ps1 -Target usuario@notebook -Install` copia o código para o
notebook, compila lá no container e instala o receptor em `~/.local/bin`.

O código fica em dois crates: `crates/core` tem o protocolo, o pareamento, a cifra, as regras de
troca de tela e a aceleração do mouse, sem depender de sistema; `crates/app` tem o Windows
(interface, captura, DDC) e o Linux X11 (receptor, captura, DDC).

</details>

## Licença e suporte

Distribuído sob a licença MIT. Veja [LICENSE](LICENSE).

O Lan Kvm é mantido por uma pessoa só. Issues são bem-vindas e lidas, mas sem prazo garantido de
resposta. Falhas de segurança devem ser avisadas em particular, como diz o [SECURITY.md](SECURITY.md).

Um relato de problema fica bem mais fácil de resolver com o log das duas máquinas:
`%LOCALAPPDATA%\lan-kvm\lan-kvm.log` no PC e `~/.local/state/lan-kvm/recv.log` no notebook. Eles têm
endereços IP e nomes de monitores, então confira antes de anexar.

MIT © [Gabriel Griffo](https://github.com/gabrielgriffo)
