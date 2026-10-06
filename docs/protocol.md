# Protocolo app ↔ EA

O app (Rust) escuta em `127.0.0.1:47011`; o EA `TerminalBridge` (`mql5/TerminalBridge.mq5`) conecta
nele, porque sockets do MQL5 só fazem conexões de saída. Uma conexão por vez: uma nova substitui a anterior.

Cada mensagem é um objeto JSON numa linha (`\n`), com o tipo no campo `"t"`. Linhas que não fazem
sentido são relatadas e ignoradas, nunca derrubam a conexão. Preços seguem os dígitos do símbolo;
horários são do servidor do MT5 (barras em segundos, ticks em milissegundos).

## App → EA

| `t` | Campos | Efeito |
|---|---|---|
| `history` | `symbol`, `tf` (`M1`…`D1`, `W1`), `count` | Envia `symbol` e `bars`. Pedidos ficam numa fila; os que o terminal ainda carrega são tentados por ~10 s |
| `subscribe` | `symbols: [..]` | Substitui a lista de símbolos cujos ticks são transmitidos |
| `order` | `id`, `symbol`, `side` (`buy`/`sell`), `kind` (`market`/`limit`/`stop`), `volume`, `price`, `sl`, `tp` | Nova ordem; `price` só vale para limite/stop; `sl`/`tp` = 0 é sem |
| `close` | `id`, `ticket` | Fecha a posição inteira a mercado |
| `cancel` | `id`, `ticket` | Remove a ordem pendente |
| `flatten` | `id`, `symbol` | Cancela as ordens e fecha as posições do símbolo, de qualquer origem |
| `probe` | `id`, `symbol`, `tf`, `indicator`, `buffer`, `count` | Diagnóstico: últimos valores de um buffer do indicador (nome curto começando com `indicator`) no gráfico do MT5 desse símbolo/timeframe |
| `objects` | `id`, `symbol`, `tf`, `prefix` | Diagnóstico: texto e preço dos objetos do gráfico cujo nome começa com `prefix` |

`id` é escolhido pelo app e volta em `trade_result`. `flatten` gera um resultado por requisição enviada
(ou um só, `"nada a zerar"`).

## EA → app

| `t` | Campos |
|---|---|
| `hello` | `symbol`, `digits`, `server`, `login`, `account` (`demo`/`contest`/`real`), `version` (protocolo; o app avisa se for mais antigo) |
| `symbol` | `symbol`, `digits`, `tick_size`, `vol_min`, `vol_max`, `vol_step` |
| `bars` | `symbol`, `tf`, `digits`, `bars: [[time, open, high, low, close, tick_volume], ..]` |
| `tick` | `symbol`, `time_msc`, `bid`, `ask`, `volume` (volume real; 1 por tick em CFD) |
| `account` | `balance`, `equity`, `margin_free`, `currency`, `trade_allowed` |
| `positions` | `positions: [{ticket, symbol, side, volume, price, sl, tp, profit}, ..]` (todas as posições da conta) |
| `orders` | `orders: [{ticket, symbol, side, kind, volume, price, sl, tp}, ..]` (todas as pendentes) |
| `trade_result` | `id`, `ok`, `retcode` (MT5), `msg`, `ticket`, `price` |
| `probe` | `id`, `indicator`, `buffer`, `times: [..]`, `values: [..]` (`null` = vazio) |
| `objects` | `id`, `items: [{name, text, price}, ..]` |
| `error` | `msg` |

`account`, `positions` e `orders` são retratos completos, enviados na conexão e depois só quando mudam
(verificado a cada 100 ms e a cada transação). `profit` inclui swap.

## Execução

- Ordens saem por `OrderSendAsync`: o EA não bloqueia esperando a corretora; o resultado chega em
  `OnTradeTransaction` (`TRADE_TRANSACTION_REQUEST`) e vira `trade_result`.
- Preenchimento pelo que o símbolo aceita: FOK, senão IOC, senão RETURN.
- Ordens do app levam o número mágico `InpMagic` (47011) e o comentário `mt5-terminal`.
- Ticks: `CopyTicks` a cada `InpTimerMs` (10 ms) e em `OnTick`, desde o último enviado de cada símbolo.

## Requisitos no MT5

1. **Ferramentas → Opções → Expert Advisors**: marcar *Permitir WebRequest para as URLs listadas* e
   adicionar `127.0.0.1` (o MT5 exige isso também para `SocketConnect`).
2. Arrastar `MT5Terminal/TerminalBridge` para qualquer gráfico, com *Permitir Algo Trading*.
3. Botão **Algo Trading** ligado na barra do MT5; sem ele o app mostra "Algo Trading desligado no MT5".

## Segurança

- Em conta **REAL** as ordens ficam travadas até marcar *Armar conta REAL* na boleta, a cada sessão
  (e de novo depois de qualquer desconexão).
- A fonte **Sintético** tem uma corretora simulada (hedge, 10 000 USD, spread 0,5) para testar a
  interface sem enviar nada ao MT5.

## Indicadores e histórico

Os indicadores vêm de um preset opcional (`src/studies.rs`). O histórico vem aos poucos
(`src/history.rs`): primeiro bloco pequeno de cada série (gráfico: 1000 candles), depois blocos
anteriores com `before` quando a tela chega perto do candle mais antigo ou quando o preset pede
aquecimento ou séries extras. A cada minuto o app rebusca os 3 últimos candles de cada série.

`probe` e `objects` servem para um preset conferir seus cálculos contra os indicadores rodando no
gráfico do MT5 (o app com preset aceita `--verify ARQUIVO`).
