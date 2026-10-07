# MT5 Terminal

**Gráficos e ordens em Rust, rápidos como o mercado pede, com o MetaTrader 5 só como ponte com a corretora.**

O MT5 continua fazendo o que faz bem: falar com a corretora. Gráfico, boleta e operação pelo gráfico
ficam num app nativo (egui + wgpu) que abre na hora, desenha só o que está na tela e nunca trava um
quadro. Um Expert Advisor (`mql5/TerminalBridge.mq5`) liga os dois por TCP local e repassa cotações,
histórico, posições e ordens.

![Gráfico com posição, stop, alvo e ordem pendente; boleta à direita](docs/img/hero.png)

## Destaques

- **Rápido de verdade.** O primeiro bloco de histórico pinta em cerca de 1 s; o resto chega aos poucos,
  conforme você volta no tempo ou dá zoom out. Indicadores incrementais: um tick custa nanossegundos.
- **Operação pelo gráfico, como no ProfitChart.** Shift compra, Ctrl vende, o clique posiciona; stop,
  alvo e ordens se arrastam; o × de cada linha fecha, cancela ou remove.
- **Operações predefinidas.** Volume, stop e alvo (em % da entrada ou em pontos) prontos num clique,
  já desenhados na ordem que segue o ponteiro.
- **Boleta completa.** Mercado, limite e stop, ZERAR, INVERTER (netting e hedge), BE e atalhos de teclado.
- **Seguro por padrão.** Conta REAL travada até você armar a boleta; a senha fica só no MT5; fonte
  **Sintético** com corretora simulada para testar sem risco.
- **Configurável por você ou por um agente.** Tudo num `config.toml` comentado, aplicado na hora, e um
  comando `ctl` que nunca envia ordens.

## Operação pelo gráfico

![Shift segurado: a ordem limite segue o ponteiro com stop e alvo da operação predefinida](docs/img/ghost.png)

- Segure **Shift** (compra) ou **Ctrl** (venda): a ordem acompanha o ponteiro, já com o volume, o stop
  e o alvo da operação predefinida, e o clique a posiciona (limite do lado favorável, stop do outro).
- Com **Alt** segurado, arraste a linha da posição para o lado do ganho (alvo) ou da perda (stop), ou
  mova ordens, stops e alvos já posicionados.
- O **×** de cada linha (ou Delete com o mouse sobre ela) fecha a posição com seu stop e alvo, cancela
  a ordem ou tira só o stop/alvo. O botão direito abre um menu de ordens.
- Atalhos: **Ctrl+Shift+B/S** compra/venda, **Z** zera, **R** inverte, **E** breakeven, mesmo com a
  boleta recolhida.

Tudo passa pelas mesmas travas da boleta.

## Gráfico

![Modo Cruz medindo +1,02% em 36 barras](docs/img/measure.png)

- Bid e ask com linhas e etiquetas no eixo, spread na legenda, contagem regressiva do candle.
- Três modos de cursor: **Seta**, **Mão** (arrastar o gráfico) e **Cruz** (clique e arraste para medir
  a variação em % e a distância em barras; Esc limpa).
- Teclado: ←/→ movem, +/− dão zoom.
- O preço fica na frente de tudo por padrão; a ordem das camadas (níveis, indicadores, posições, preço)
  muda no menu **Camadas**.
- Mais espaço quando precisar: a boleta recolhe numa faixa (»/«) e o painel de indicadores minimiza.
- Troca entre conta demo e real pela barra de status: o app reabre o MT5 na conta escolhida e pede
  confirmação antes da REAL.

## Cores

![Janela Cores com predefinições e cores de candle](docs/img/colors.png)

Como a aba de cores do MT5: fundo, painéis, grade, texto, destaque, alta/baixa e candles (corpo e
contorno separados; corpo da cor do fundo = candle vazado). Predefinições prontas (Padrão escuro,
MetaTrader clássico, Claro, TradingView), aplicadas na hora.

## Desempenho

Medido num notebook (Hyprland, 1920×1080), 20 mil candles e 20 ticks por segundo:

| | |
|---|---|
| Quadro (interface) | ~0,8 ms em média, p99 < 2 ms (orçamento de 60 Hz: 16,7 ms) |
| Parado | ~4% de CPU, ~80 MB de memória |
| Tick sem candle novo | ~180 ns nos indicadores |
| Candle novo | ~0,25 ms |
| Carga completa de 20 mil candles | ~60 ms, fora da thread da interface |

Para conferir no seu ambiente: `cargo test --release -- --ignored --nocapture bench`, ou rode o app
com `MT5_TERMINAL_PERF=1` para ver quadros/s, ticks/s e o tempo de cada quadro a cada 5 s.

## Instalação

Requer Rust (edition 2024) e o MetaTrader 5 (no Linux, via Wine).

```sh
cargo build --release
target/release/mt5-terminal              # abre o MT5 se ele não estiver aberto
target/release/mt5-terminal --synthetic  # sem MT5: dados e corretora simulados
```

No MT5:

1. Compile e instale o EA: `tools/build-ea.sh` (ou abra `mql5/TerminalBridge.mq5` no MetaEditor).
2. **Ferramentas → Opções → Expert Advisors**: permita WebRequest para `127.0.0.1`.
3. Anexe `MT5Terminal/TerminalBridge` a um gráfico e ligue o **Algo Trading**.

Uma instância só: abrir de novo traz a janela existente para a frente. Protocolo e detalhes em
[docs/protocol.md](docs/protocol.md).

## Configuração

`~/.config/mt5-terminal/config.toml`, comentado e aplicado na hora ao salvar (`mt5-terminal config
init` cria, `config check` valida). Com o app aberto, `mt5-terminal ctl help` lista comandos como
`ctl symbol UsaInd`, `ctl tf H1`, `ctl front price` e `ctl state`. Agentes (Claude Code) têm uma skill
em `.claude/skills/configure-mt5-terminal`. Nada disso envia ordens.

## Indicadores

O app não traz indicadores próprios: um preset opcional em `preset/` (outro repositório) é compilado
quando a pasta existe, pela interface de [src/studies.rs](src/studies.rs). Sem ele, o app roda com o
gráfico limpo, como nas imagens acima. O preset pode substituir o volume do rodapé (ex.: delta de
volume), marcar um preço por candle (ex.: POC) e ter um painel sob o gráfico.

O preset inclui Fibonacci automático no **M5, M15, H1 e D1**, integrado às convergências do mapa de
suportes e resistências. Usa o último movimento entre topo e fundo confirmados por 2 candles fechados
de cada lado, dentro dos últimos 300 candles fechados de cada série. Retrações padrão: **23,6%, 38,2%,
50%, 61,8% e 78,6%**. Cada etiqueta identifica timeframe e percentual, alinhada à extrema esquerda do
gráfico; ao passar o mouse, aparecem todas as referências com seus preços. Vários percentuais do mesmo
Fibonacci contam como uma referência na convergência; timeframes diferentes contam separadamente. O
mapa mantém até três níveis de cada lado do preço, priorizando convergências. Sem um par de pivôs
confirmado, aquela série aguarda confirmação. Ajustes em `[fibonacci]` no config.toml: `enabled`,
`lookback` (20–2000), `pivot_bars` (1–10) e `levels` (frações de 0 a 1).

## Segurança

A ponte escuta só em `127.0.0.1`. O app não guarda credenciais: o login na corretora é feito no MT5.
Use uma conta demo ou a fonte Sintético para testar ordens.

## Licença

[MIT](LICENSE)
