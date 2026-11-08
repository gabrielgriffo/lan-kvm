# Auditoria de segurança: lan-kvm, 2026-11-07

Escopo: commit `b9d0849` (árvore limpa), todo o repositório. As correções entraram no commit seguinte.

## Modelo de ameaças

- **Ativos:**
  - a capacidade de digitar e clicar na outra máquina, o que equivale a executar código como o
    usuário logado;
  - o conteúdo do clipboard;
  - o segredo do pareamento;
  - a disponibilidade do controle.
- **Atacantes:**
  - outra máquina na mesma rede, sem o segredo;
  - quem consegue interceptar ou alterar o tráfego, inclusive durante o pareamento;
  - um par pareado que se torna malicioso;
  - outros processos e usuários locais.
- **Entradas:**
  - o servidor TCP do receptor (`0.0.0.0:24801`) e a sessão cifrada nos dois sentidos;
  - o arquivo do segredo;
  - o clipboard;
  - os scripts de instalação e o autostart.

## Evidência automatizada

- **`gitleaks`**, sobre o histórico (10 commits) e os arquivos versionados ou não ignorados: nenhum
  vazamento. Os 14 alertas de uma primeira varredura vinham de `target/`, a saída de build.
- **`cargo deny check advisories`:** um único aviso, `RUSTSEC-2024-0370` (`proc-macro-error` sem
  manutenção, não é uma vulnerabilidade). Ele chega via `gtk`, que o `tray-icon` só usa no Linux;
  `cargo tree` confirma que o pacote não entra em nenhum dos dois builds reais.
- **`cargo audit`:** não está instalado.
- **Alertas do GitHub:** o repositório não tem remoto nem CI, então não há alertas a consultar.
- **`clippy`** com lints de truncamento e indexação: as pistas foram conferidas manualmente e estão
  todas protegidas por limites anteriores.

## Achados e correções

A revisão manual foi feita por um agente independente. Cada achado foi refeito no código antes de
ser aceito.

| # | Severidade | Achado | Correção | Verificação |
|---|---|---|---|---|
| A1 | Medium | Um atacante na rede, sem o segredo, segurava o receptor indefinidamente com uma conexão lenta (o timeout valia por leitura, e o servidor atendia uma conexão por vez) | Uma thread por conexão não autenticada, prazo total de 10 s (`net::Deadline`), no máximo 4 conexões pendentes por IP e 32 no total | Testes unitários; ponta a ponta: com 3 conexões lentas abertas o PC conectou em 0,6 s, elas foram cortadas em cerca de 11 s, e a 5ª e a 6ª conexão do mesmo IP foram recusadas |
| A2 | Medium | Com o notebook sem pareamento, as tentativas de código não tinham limite total (1 a cada 10 s, sem teto) | Bloqueio depois de 5 códigos errados até reiniciar o receptor; um pareamento por vez; só conta como falha a confirmação de chave que falhou | Teste unitário do `Gate`; ponta a ponta: 5 erros e o 6º pedido bloqueado |
| A3 | Low | Um par pareado lia ou trocava o clipboard do outro lado a qualquer momento (com `Leave` ou `TakeOver` repetidos) | Clipboard só é enviado em transições originadas localmente; o recebido só é aplicado em `Enter`, num `GiveBack` com captura ativa, num `TakeOver` aceito ou na resposta a um `Leave`; mais de 1 MiB é descartado | Testes de `SessionState`; ponta a ponta do clipboard nos dois sentidos |
| b1 | Low | Um PC pareado mandava `Motion` sem `Enter`, empurrava o ponteiro do notebook até a borda e disparava a captura, recebendo então tudo o que fosse digitado no notebook | O notebook só injeta movimento, botões, roda e teclas depois do `Enter` | Teste de `SessionState` |
| A4 | Low | Overflow em `evdev + 8` com valor vindo do par (panic em build debug) | `checked_add` | Revisão |
| A5 | Low | Um segredo ilegível no notebook era tratado como "não pareado" e reabria o pareamento | `keystore::load` devolve erro, e o receptor recusa o pareamento e a sessão | Teste unitário; ponta a ponta com o segredo corrompido |
| A6 | Low | O notebook gravava o segredo antes de saber se o PC tinha terminado o pareamento | O PC envia uma confirmação cifrada, e o notebook só grava depois de recebê-la | Teste `responder_does_not_accept_when_initiator_never_confirms` |
| A7 | Low | O código de pareamento ia para um log com permissões padrão | Log em `~/.local/state/lan-kvm` com 0600 (`umask 077` no autostart) | Ponta a ponta: `-rw-------`, diretório `drwx------` |
| A8 | Informational | Enquanto o notebook controla o PC, não havia escape no lado do PC | Ctrl+Alt+Backspace no teclado do PC devolve o controle | Ponta a ponta |
| A9 | Informational | Byte de modo fora do transcript Noise; `Delta` sem limite; mensagem inesperada registrada no log com o conteúdo | O modo entra no *prologue*; `Delta` limitado a ±2048; o log não traz o conteúdo da mensagem | Teste `different_modes_fail_the_handshake` |

Durante os testes apareceu também um problema de robustez: a captura anunciava o `TakeOver` antes de
conseguir o grab. Agora o grab vem primeiro. Além disso, a captura pede XInput 2.2, cujos eventos
continuam chegando mesmo quando outro programa (por exemplo, o modo Atividades do GNOME) segura o
mouse.

## Fronteiras revisadas sem achados

- **Pareamento e canal cifrado:**
  - o código é gerado com CSPRNG, sem viés;
  - um MITM no pareamento tem um único palpite, sem ataque offline;
  - nonces por direção, com replay rejeitado (teste);
  - chaves efêmeras por sessão;
  - tamanhos dentro do limite do Noise.
- **Parsing:**
  - tamanho exato por tipo de mensagem;
  - limite de 16 MiB antes de bufferizar;
  - até 64 KiB alocados antes da autenticação.
- **Segredo:** diretório 0700 e arquivo 0600 no Linux; DPAPI do usuário no Windows.
- **FFI Win32:** tamanhos de buffer em `GetRawInputData`, `GetUserObjectInformationW`,
  `CreateCursor`, `RegSetKeyValueW` e `SendInput`; liberação dos buffers do DPAPI com `LocalFree`.
- **Comandos externos:** `notify-send` recebe os argumentos sem shell; o `.desktop` rejeita
  caracteres especiais no caminho.

## Pendências sem confirmação

- **Fila de erros X11 no injetor:** que os erros de requisições inválidas se acumulem na fila do
  `RustConnection` não foi confirmado. Mesmo assim, o injetor passou a esvaziar a fila depois de cada
  mensagem.

## Risco residual e escopo não testado

- **Par pareado:** quem tem o segredo pode digitar e clicar na outra máquina, que é o propósito do
  programa. Quem tiver root no notebook corporativo, ou rodar como o mesmo usuário, lê o segredo e
  pode controlar o PC. Uma opção futura é perguntar no PC antes de aceitar o controle vindo do
  notebook.
- **Bloqueio do pareamento:** um atacante na rede consegue bloquear novos pareamentos (o que exige
  reiniciar a sessão do notebook), mas não consegue parear.
- **Mutex de instância única:** o mutex nomeado `Local\lan-kvm-sender` pode ser criado antes por
  outro processo da mesma sessão, impedindo o programa de abrir.
- **Não houve teste de invasão** a partir de outra máquina da rede. Os testes adversariais rodaram a
  partir do próprio PC.
