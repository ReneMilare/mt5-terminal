---
name: configure-mt5-terminal
description: Configure or inspect the MT5 Terminal app (chart symbol/timeframe, symbol list, drawing layers, indicators on/off, colors, first history chunk, MetaTrader 5 auto-start and command). Use when the user asks to change how MT5 Terminal looks or behaves, or asks what it is showing. Not for sending orders.
---

# Configurar o MT5 Terminal

Tudo que é configurável fica em um arquivo TOML comentado e o app em execução o aplica sozinho ao
salvar. Para ações imediatas existe `mt5-terminal ctl`, como o `hyprctl` do Hyprland.

## Arquivo: `~/.config/mt5-terminal/config.toml`

- Caminho exato: `mt5-terminal config path`. Se não existir: `mt5-terminal config init` cria com os
  padrões comentados. Referência completa de chaves e padrões: `mt5-terminal config defaults`.
- Edite só as chaves necessárias, preservando comentários e o resto do arquivo.
- **Sempre valide depois de editar:** `mt5-terminal config check`. `erro:` = o app mantém a
  configuração anterior até você corrigir; `aviso:` = chave desconhecida ou valor ajustado.

| Chave | Valores |
|---|---|
| `chart.symbols` | lista de símbolos do seletor (nomes exatos do MT5) |
| `chart.symbol`, `chart.timeframe` | ao abrir; timeframes `M1 M5 M15 M30 H1 H4 D1 W1` |
| `chart.layers` | as 4 camadas, de trás para a frente: `levels`, `indicators`, `trades`, `price` (padrão: preço na frente) |
| `chart.show_studies` | indicadores do preset |
| `chart.first_bars` | candles do primeiro bloco (100–20000); mais antigos vêm conforme o usuário volta no tempo |
| `chart.cursor` | `arrow` (seta), `hand` (mão para arrastar), `cross` (cruz: arrastar mede % e barras; Esc limpa) |
| `fibonacci.enabled` | Fibonacci M5/M15/H1/D1 no mapa do preset |
| `fibonacci.lookback`, `fibonacci.pivot_bars` | 20–2000 candles fechados de busca; 1–10 candles fechados de cada lado para confirmar pivôs (padrão 300 / 2) |
| `fibonacci.levels` | 1–12 frações únicas entre 0 e 1; padrão `[0.236, 0.382, 0.5, 0.618, 0.786]` |
| `[studies.<indicador>]` | opções de cada indicador do preset (só as alteradas). Os indicadores, as opções e os valores atuais estão em `ctl state` → `indicators`; cores `"#rrggbb"`, timeframes como lista `["M5", "H1"]`, escolhas pela chave. Apagar a tabela volta ao padrão. O usuário também edita pelo gráfico: botão direito sobre o indicador → Editar |
| `mt5.auto_start` | abrir o MetaTrader 5 junto se ele estiver fechado |
| `mt5.command` | comando que abre o MT5 (lista); vazio = como o atalho do MT5 |
| `colors.background/panel/grid/text/accent` | `"#rrggbb"`: fundo do gráfico, painéis, grade, texto, destaque |
| `colors.up/down` | alta/baixa (botões, preço, volume) |
| `colors.candle_up/candle_down`, `colors.wick_up/wick_down` | corpo e contorno/pavio dos candles; corpo da cor do fundo = vazado |
| `[[presets]]` | `name`, `volume`, `stop`, `target` (0 = sem), `unit` (`percent` da entrada ou `points`): operações predefinidas da boleta |
| `ticket.preset` | nome da operação ativa; vazio = Manual |
| `ui.ticket_open`, `ui.pane_open` | boleta aberta / recolhida; painel do preset aberto / minimizado |
| `[[accounts]]` | `name`, `login`, `server`, `kind` (`demo`/`real`): contas da troca pela barra de status; o app as anota sozinho; **nunca** grave senha |

O app relê o arquivo em até meio segundo enquanto a janela está sendo desenhada; com a janela num
workspace escondido, aplica quando ela voltar a aparecer.

## Comandos: `mt5-terminal ctl <comando>`

Respondem na hora mesmo com a janela escondida. Saída `erro: ...` (código 1) em falha.

- `ctl state` — JSON: símbolo, timeframe, fonte, conexão, conta (`kind`: `demo`/`real`), candles,
  camadas, indicadores, Fibonacci (âncoras/níveis por timeframe), posições, ordens, status. Use para conferir o efeito de uma mudança.
- `ctl symbol <S>`, `ctl tf <TF>` — muda o gráfico agora (não altera o padrão do arquivo).
- `ctl studies on|off`, `ctl front <camada>`, `ctl layers a,b,c,d` — gravam no config.toml.
- `ctl reload` — relê o arquivo. `ctl config` — caminho. `ctl help` — lista.

## Limites

- `ctl` e o arquivo **não enviam ordens** nem trocam de conta, por desenho (a troca é na barra de status, e para REAL pede confirmação). Operar é com o usuário, na boleta ou no
  gráfico do app.
- O app tem uma instância só; se `ctl` disser que ele não está aberto, peça ao usuário para abri-lo
  (ou rode `mt5-terminal`, que também abre o MT5 se `mt5.auto_start`).
- A porta da ponte com o MT5 (127.0.0.1:47011) não é configurável aqui: precisa casar com o EA.
