# Segurança

O Lan Kvm leva o teclado, o mouse, o clipboard e, se ligado, o som e o microfone de uma máquina para
outra. Quem controla essa conexão consegue digitar na outra máquina, o que equivale a usá-la como o
usuário logado. Este arquivo diz o que a conexão protege, o que não protege e como avisar de uma falha.

## O que a conexão protege

- **Só o par pareado conecta.** O pareamento é feito uma vez, com um código de 6 dígitos que o
  notebook mostra e você digita no PC. O código passa por uma troca SPAKE2 e nunca trafega pela rede;
  uma gravação do pareamento não permite testar códigos fora do ar.
- **Tentativas de adivinhar o código são limitadas.** Cada pedido de pareamento vale um único palpite,
  o notebook aceita no máximo um pedido a cada 10 s e, depois de 5 códigos errados, recusa novos
  pareamentos até a próxima sessão. Depois de pareado, o notebook recusa novos pareamentos até você
  rodar `lan-kvm unpair` nele.
- **Todo o tráfego é cifrado e autenticado.** Cada conexão faz um handshake Noise `NNpsk0` (X25519,
  ChaCha20-Poly1305, SHA-256) com o segredo do pareamento. Mensagens alteradas, repetidas ou fora de
  ordem derrubam a conexão. As chaves efêmeras fazem com que um segredo vazado depois não decifre
  conexões gravadas antes.
- **O segredo nunca é enviado.** No PC ele fica em `%APPDATA%\lan-kvm\token.bin`, protegido pelo DPAPI
  do usuário do Windows; no notebook, em `~/.config/lan-kvm/token`, com permissão 0600.
- **Conexões não autenticadas não seguram o receptor.** Elas têm 10 s para completar o handshake, com
  no máximo 4 ao mesmo tempo por endereço e 32 no total.
- **Cada lado só aceita o que faz sentido no momento.** O notebook só injeta teclado e mouse depois que
  o PC entra nele, e o clipboard recebido só é aplicado numa troca de controle de verdade, com até
  1 MiB de texto. Os dois lados recusam uma versão de protocolo diferente da sua.
- **Nada sai da sua rede.** O PC fala direto com o notebook. Não há conta, servidor intermediário,
  telemetria nem verificação de atualizações.

## O que não protege

- **Um par pareado pode tudo.** Quem tem o segredo pode digitar e clicar na outra máquina: é o
  propósito do programa. Quem tiver root no notebook, ou rodar como o mesmo usuário, lê o segredo e
  pode controlar o PC, e o mesmo vale ao contrário para o seu usuário no Windows.
- **O receptor escuta na rede inteira.** No notebook, ele aceita conexões em todas as interfaces, na
  porta TCP 24801. Sem o segredo ninguém controla nada, mas qualquer máquina da rede vê que a porta
  está aberta e quando há tráfego. Use o Lan Kvm em redes em que você confia, ou restrinja a porta ao
  endereço do PC no firewall do notebook.
- **Alguém na rede consegue bloquear o pareamento.** Errando o código 5 vezes, um atacante impede
  novos pareamentos até a próxima sessão do notebook, mas não consegue parear.
- **O microfone e o som viajam pela conexão.** Com "Usar o fone e o microfone do PC no notebook"
  ligado, o áudio vai cifrado como o resto, mas o notebook passa a ouvir o microfone do PC enquanto
  estiverem conectados.
- **Os logs têm dados da sua rede.** `%LOCALAPPDATA%\lan-kvm\lan-kvm.log` e
  `~/.local/state/lan-kvm/recv.log` registram endereços IP e nomes de monitores, e o do notebook mostra
  o código de pareamento enquanto ele vale. Eles nunca registram o que é digitado nem o conteúdo do
  clipboard.
- **Não houve teste de invasão a partir de outra máquina.** A auditoria de 2026-11-07, em
  `docs/auditoria-seguranca-2026-11-07.md`, revisou o código e rodou testes adversariais a partir do
  próprio PC.

## Como avisar de uma falha

Não abra uma issue pública. Use o aviso privado de vulnerabilidade do GitHub, na aba **Security** do
repositório, em **Report a vulnerability**. Diga a versão do Lan Kvm nos dois lados, o que acontece e
como reproduzir. Antes de anexar logs, confira se não há neles nada que você não queira compartilhar.

O projeto é mantido por uma pessoa só. O aviso é lido, mas não há prazo garantido para a resposta ou
para a correção.

## Versões cobertas

Só a versão mais recente recebe correções de segurança. O PC e o notebook precisam estar na mesma
versão, porque cada um recusa uma versão de protocolo diferente da sua.
