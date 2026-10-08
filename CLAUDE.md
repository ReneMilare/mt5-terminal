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
  de mesma cor. Meça antes de afirmar ganho: `cargo test --release -- --ignored --nocapture bench`;
  no app, `MT5_TERMINAL_PERF=1` imprime a cada 5 s quadros/s, ticks/s e o tempo de cada quadro.

## Gráfico

- O rodapé mostra o volume, ou o que o preset puser no lugar (`Studies::volume`, ex.: delta de volume).
  O delta por candle: exato do EA para os fechados (comando `delta`, em fatias), ao vivo pelos ticks
  para o em formação (`model::Deltas`), e o EA é chamado de novo a cada candle que fecha.
- Marcas por candle sobre os candles (`Studies::marks`, ex.: POC): traço que acompanha a largura do
  candle (`chart::Marks`). A POC vem junto com o delta (`row` do preset no comando `delta`).

- Indicadores editáveis pelo gráfico: botão direito sobre a linha, nível, marca, faixa de volume, fundo
  ou painel de um indicador → "Editar …" (ou o submenu Indicadores); a janela aplica na hora e grava só o
  que difere do padrão em `[studies.<indicador>]` (`studies::StudyParams`, opções declaradas pelo preset).
  Cada elemento desenhado leva a chave do indicador (`study`) para o clique direito (`chart::study_at`).
  O preset também pode pintar o fundo por candle (`Shading`), desenhar faixas finas na base do gráfico
  (`Ribbon`, uma classe por candle em cada linha; o volume sobe para ficar em cima delas) e pôr uma linha
  de estado sob a legenda.
- O **preço fica na frente de tudo** por padrão (`Layer::DEFAULT`, de trás para a frente: níveis,
  indicadores, posições/ordens, preço). O usuário muda a ordem no menu **Camadas**; a escolha fica no
  `config.toml` (`chart.layers`). O volume fica sempre no fundo.
- Bid e ask têm linhas e etiquetas identificadas na camada preço, com spread em preço na legenda;
  etiquetas próximas se separam sem deslocar as linhas. Cotações aparecem mesmo com a boleta travada;
  `ChartData::can_trade` controla apenas as prévias de ordem e o menu de operação.
- Operações predefinidas (`[[presets]]`: volume, stop e alvo em % da entrada ou em pontos; a ativa em
  `[ticket] preset`): escolhidas na boleta, editadas na janela "Operações predefinidas"; com Shift/Ctrl a
  ordem que segue o ponteiro já mostra o stop e o alvo (`trading::Bracket`). Volumes e preços vão à
  corretora exatamente na grade do símbolo (`round_to`, sem ruído de ponto flutuante).
- Cores (janela **Cores**, como a aba de cores do MT5): fundo, painéis, grade, texto, destaque, alta/baixa
  e candles (corpo e contorno/pavio separados; corpo ≠ contorno desenha borda), com predefinições;
  aplicadas na hora e gravadas em `[colors]` (`COLOR_KEYS` em `settings.rs`).
- Teclado: ←/→ movem, +/− zoom (fora de campos de texto). Atalhos de ordem: Ctrl+Shift+B/S (compra/venda), Z (zerar), R (inverter), E (breakeven), também com a boleta recolhida (`Trading::shortcuts`).
- Cursor por ícones na barra superior: Seta aponta, Mão arrasta o gráfico, Cruz mede ao clicar e arrastar (% sobre
  o preço inicial e distância entre barras; a mesma barra = 0). Soltar mantém a medição; Esc, um clique
  no gráfico ou troca de cursor/símbolo/timeframe limpa. Seleção em `chart.cursor` (`arrow/hand/cross`).
- Mais espaço para o gráfico: a boleta recolhe numa faixa (» / «) e o painel do preset minimiza numa
  faixa ("minimizar" / clique na faixa); o estado fica em `[ui]` (`ticket_open`, `pane_open`).
- Como no ProfitChart: segurando Shift (compra) ou Ctrl (venda) a ordem acompanha o ponteiro, já no
  preço do tick, e o clique a posiciona (limite do lado favorável, stop do outro); arrastar a linha de
  uma posição **com Alt segurado** para o lado do ganho põe o alvo, para o da perda o stop (compra:
  cima = alvo); com Alt, linhas de ordem, stop e alvo se arrastam; sem Alt, o modo Mão move o gráfico
  e o modo Cruz mede. O × da linha nos modos Seta/Mão (ou Delete com o mouse sobre ela) fecha a posição (o
  stop e o alvo dela vão junto), cancela a ordem ou tira só o stop/alvo; botão direito abre um menu.
  Tudo sob as mesmas travas da boleta.

## Indicadores

O app não traz indicadores: um preset privado em `preset/` (repositório próprio, ignorado por este)
entra na compilação quando a pasta existe (`build.rs` → `cfg(has_preset)`) e implementa a interface de
`src/studies.rs` (inclusive `params`/`configure`, as opções editáveis de cada indicador). Sem a pasta, o app compila e roda sem indicadores; teste as duas formas. Detalhes do
preset ficam em `preset/CLAUDE.md`. `[fibonacci]` configura os pivôs/retrações do mapa do preset;
`Studies::new` recebe as opções, `configure_fibonacci` aplica alterações e `fibonacci_state` expõe as
âncoras e os níveis no `ctl state`.

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
  o app o abre como o atalho dele (`config.toml`: `mt5.auto_start`, `mt5.command`).
- Troca de conta pela barra de status: o app fecha o MT5 pela janela (`hl.dsp.window.close`), reabre
  com `/config:` (login e servidor, Algo Trading mantido) e devolve a janela ao workspace dela. Senha
  nunca no app: o MT5 usa a que salvou. Contas em `[[accounts]]`, aprendidas a cada conexão; trocar para
  REAL pede confirmação. O `ctl` não troca de conta.

## EA

- `tools/build-ea.sh` compila com o MetaEditor e instala em `MQL5/Experts/MT5Terminal`, usando o mesmo
  Wine que serve o prefixo `~/.mt5` (nunca dois Wines no mesmo prefixo; sem MT5 aberto, o `wine` do
  sistema, como o atalho do MT5).
- O MT5 **não** recarrega o EA sozinho, mas carrega o `.ex5` novo ao iniciar: depois de recompilar,
  feche o MT5 pela janela (`hl.dsp.window.close`) e abra de novo (o app abre o MT5 se ele estiver
  fechado) — não é preciso remover e anexar o EA.
  Mudou o protocolo? Suba `BRIDGE_VERSION` nos dois lados; o app avisa quando o EA é mais antigo.

## Testes

- `cargo test`, `cargo clippy --release` sem avisos.
- **Feche toda instância de teste** ao terminar (`pkill -x mt5-terminal` só nas que você abriu): uma
  instância esquecida segura a porta e a do usuário não carrega.
- Testes visuais sempre num **workspace vazio** do Hyprland (confira com `hyprctl workspaces` antes:
  o MT5 muda de workspace entre sessões) e confira que o foco está no app antes de mandar teclas.
- Nunca envie ordens de teste por uma conta **REAL** (o app mostra o tipo da conta na barra de status):
  use uma conta demo do MT5 ou a fonte **Sintético** (corretora simulada).
