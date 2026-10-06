# MT5 Terminal

Terminal de gráficos e ordens em Rust (eframe/egui + wgpu). O MetaTrader 5 é só a ponte com a
corretora: o EA `mql5/TerminalBridge.mq5` conecta no app por TCP local (`docs/protocol.md`).

## Prioridade: rápido e performático

É mercado financeiro: o app tem que abrir na hora e nunca travar um quadro. Toda mudança é avaliada
por isso antes de qualquer outra coisa.

- **Histórico aos poucos, nunca tudo de uma vez.** Cada série começa com um bloco pequeno (gráfico:
  `FIRST_BARS`, poucas telas) para pintar já. Blocos mais antigos (`history::Loader`, um pedido por
  série de cada vez, comando `history` com `before`) só vêm quando algo precisa: a tela chegou perto do
  candle mais antigo (arrastar para trás, zoom out, teclado), o aquecimento dos indicadores
  (`Studies::warmup`) ou as séries extras que eles pedem (`Studies::needs`). Não aumente o primeiro pedido para "resolver" falta de
  dados: peça o bloco que falta quando ele for necessário.
- **Nada pesado no caminho do tick.** Indicadores são incrementais: um tick sem candle novo custa
  O(1), um candle novo processa só esse candle, histórico antigo inserido na frente só desloca os
  valores (`trend::shift_front`), sem recalcular. A passada completa roda uma vez, depois do
  aquecimento. Arquivos externos só são relidos quando mudam, nunca no tick.
- **Desenho:** só os candles visíveis, malhas únicas (`Mesh`) para candles e volume, linhas por trechos
  de mesma cor. Meça antes de afirmar ganho: `cargo test --release -- --ignored --nocapture bench`.

## Gráfico

- O **preço fica na frente de tudo** por padrão (`Layer::DEFAULT`, de trás para a frente: níveis,
  indicadores, posições/ordens, preço). O usuário muda a ordem no menu **Camadas**; a escolha fica em
  `~/.config/mt5-terminal/settings.json` (`settings.rs`). O volume fica sempre no fundo.
- Teclado: ←/→ movem, +/− zoom (fora de campos de texto). Atalhos de ordem: Ctrl+Shift+B/S (compra/venda), Z (zerar), R (inverter), E (breakeven).
- Como no ProfitChart: Shift+clique compra e Ctrl+clique vende no preço clicado (limite do lado
  favorável, stop do outro); arrastar a linha de uma posição para o lado do ganho põe o alvo, para o
  da perda o stop (compra: cima = alvo); linhas de ordem, stop e alvo se arrastam; botão direito abre
  um menu de ordens. Tudo sob as mesmas travas da boleta.

## Indicadores

O app não traz indicadores: um preset privado em `preset/` (repositório próprio, ignorado por este)
entra na compilação quando a pasta existe (`build.rs` → `cfg(has_preset)`) e implementa a interface de
`src/studies.rs`. Sem a pasta, o app compila e roda sem indicadores; teste as duas formas. Detalhes do
preset ficam em `preset/CLAUDE.md`.

## Configuração

- Tudo que o usuário ajusta mora em `~/.config/mt5-terminal/config.toml` (`settings.rs`): o texto
  padrão comentado (`DEFAULT_TOML`) é a documentação e a fonte dos padrões; o app relê ao salvar; a
  interface grava de volta preservando comentários. Nova opção = nova chave lá, com comentário.
- `mt5-terminal ctl` (`control.rs`) responde sem depender da janela (Wayland não desenha janela
  escondida): estado por retrato publicado, configurações gravadas no arquivo, símbolo/timeframe em
  fila. **Nunca envia ordens.** Skill: `.claude/skills/configure-mt5-terminal`.

## Processos

- Uma instância só: ela é dona da porta do EA (47011). Abrir de novo traz a existente para a frente e
  sai (`launcher.rs`); só `--synthetic` roda ao lado. Ao abrir, se o MetaTrader 5 não estiver rodando,
  o app o abre como o atalho dele (`settings.json`: `auto_start_mt5`, `mt5_command`).

## EA

- `tools/build-ea.sh` compila com o MetaEditor e instala em `MQL5/Experts/MT5Terminal`, usando o mesmo
  Wine que serve o prefixo `~/.mt5` (nunca dois Wines no mesmo prefixo; sem MT5 aberto, o `wine` do
  sistema, como o atalho do MT5).
- O MT5 **não** recarrega o EA sozinho: depois de recompilar, o usuário remove e anexa de novo.
  Mudou o protocolo? Suba `BRIDGE_VERSION` nos dois lados; o app avisa quando o EA é mais antigo.

## Testes

- `cargo test`, `cargo clippy --release` sem avisos.
- **Feche toda instância de teste** ao terminar (`pkill -x mt5-terminal` só nas que você abriu): uma
  instância esquecida segura a porta e a do usuário não carrega.
- Testes visuais sempre num **workspace vazio** do Hyprland (confira com `hyprctl workspaces` antes:
  o MT5 muda de workspace entre sessões) e confira que o foco está no app antes de mandar teclas.
- Nunca envie ordens de teste por uma conta **REAL** (o app mostra o tipo da conta na barra de status):
  use uma conta demo do MT5 ou a fonte **Sintético** (corretora simulada).
