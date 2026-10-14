# Fontes e evidências — pesquisa de estratégia de daytrade (2026-10-13)

Síntese da pesquisa em artigos/papers sobre como combinar os indicadores do preset
(KalmanTrend, delta/POC, LevelMap/Fibonacci, RegimePanel/GEX, SessionVWAP, opções IV)
numa estratégia de daytrade. Cada item indica o tipo de evidência.

## Honestidade inicial (por que exigir validação)

| Achado | Fonte | Evidência |
|---|---|---|
| Day traders em Taiwan perderam em média ~24 bps/dia líquidos; <1% previsivelmente lucrativo | Barber, Lee, Liu & Odean | Acadêmica |
| 97% de quem persistiu >300 dias em day trade de futuros de índice no Brasil perdeu | Chague, De-Losso & Giovannetti (2020) | Acadêmica |
| 7.846 regras de análise técnica intraday testadas nos EUA: nenhuma lucrativa após correção de data snooping | Sullivan, Timmermann & White | Acadêmica |

## 1. Filtro de tendência por Kalman

| Fonte | O que mostra | Evidência |
|---|---|---|
| Quantitativo, "Fast Trend Following" (https://www.quantitativo.com/p/fast-trend-following) | Dois Kalman (rápido/lento) + percentil da distância; NQ 1-min: 29,7% a.a., Sharpe 1,19, 16 trades/dia, **PF 1,07 e sem custos**; OOS do portfólio de 10 params positivo | Backtest sério, sem custos |
| QuanterLab, "Stochastic Kalman Trend" (https://quanterlab.com/articles/stochastic-kalman-trend) | Local linear trend em log-price; slope com threshold para evitar whipsaw; falha declarada em mercado choppy; tuning por "planalto", walk-forward, nunca a melhor célula | Metodologia/conceitual |
| PyQuantLab, Kalman + trailing (https://pyquantlab.medium.com/trend-following-with-kalman-filter-and-trailing-stops-a-rolling-backtest-2ecb47203611) | Framework rolling com saída só por trailing; sem métricas | Framework |
| Tese Toronto Met — regime-switching 2 estados + Kalman + FSM (https://rshare.library.torontomu.ca/articles/thesis/.../14651955/1) | Classificação de regime + Kalman supera modelos comparados | Tese |
| Alpha Architect — trend following em regime-switching (https://alphaarchitect.com/optimal-trend-following-rules-in-two-state-regime-switching-models/) | Trend following só vale em regime de tendência | Análise quant |

**Nosso dado interno (docs/research/kalman-tendencias-2026-10-13):** seguir a cor do M5 é
líquido negativo em todas as variantes; acerto direcional 60 min = 51–53% (sem vantagem
preditiva demonstrada). O Kalman é **contexto**, não gatilho.

## 2. Order flow: delta de volume, CVD, POC

| Fonte | O que mostra | Evidência |
|---|---|---|
| LuxAlgo, Delta Divergence (https://www.luxalgo.com/library/concept/delta-divergence/) | Divergência é **condição, não gatilho**; exige confirmação de estrutura/absorção | Conceitual |
| Bookmap, CVD (https://bookmap.com/blog/how-cumulative-volume-delta-transform-your-trading-strategy) | Absorção/continuação; "breakout sem expansão do CVD é de baixa qualidade"; combinação explícita com VWAP | Comercial/educacional |
| Trader-Dale, Volume Cluster/POC (https://www.trader-dale.com/my-best-trading-strategy-learn-how-to-trade-using-volume-profile-and-poc/) | Pullback com ordem no cluster de volume (≥15–20% do profile), stop além do cluster, alvo no extremo da perna/HVN | Anedótica (16 anos) |
| Cont, Kukanov & Stoikov (2014) (https://arxiv.org/abs/1011.6402) | OFI explica ~65–70% da variação **contemporânea** de preço | Acadêmica |
| Kolm, Turiel & Westray (2021/23) (https://papers.ssrn.com/sol3/papers.cfm?abstract_id=3900141) | Poder preditivo real do OFI, mas de ~2 mudanças de preço → flow é **timing**, não tese de swing | Acadêmica |
| "Predictive Order Flow Imbalance" (SSRN 2025) | IC +0,0044 (t=3,26) — conteúdo preditivo positivo e minúsculo | Acadêmica |
| Theissen (2001) / NBER 14158 / Chakrabarty et al. | Tick rule e Lee-Ready classificam lado com ~66–93% de acerto → **nosso delta é estimativa ruidosa** | Acadêmica |
| pedrobraiti/volume-profile-trading (https://github.com/pedrobraiti/volume-profile-trading) | 17–33 anos, walk-forward, custos, testes adversariais: Edge-to-Edge QQQ PF 2,06; volume exhaustion 1,5–2,2; **barra de sinal com volume 1,2–1,5× a média: PF 1,14 → 1,63–1,89**; 80% Rule falha; nenhum sleeve passa em todos os testes | Backtest honesto (melhor do tema) |

## 3. Fibonacci e confluência de níveis

| Fonte | O que mostra | Evidência |
|---|---|---|
| Tsinaslanidis, Guijarro & Voukelatos (2022), ESWA (https://www.sciencedirect.com/science/article/pii/S0957417421012495) | **"Sem suporte ao uso de retracções Fibonacci como regra standalone"** — ações quicam em zonas Fib e não-Fib com probabilidades semelhantes | Acadêmica (refuta) |
| "The Fibonacci Illusion" (SSRN 2026) | Golden pocket 38,2–61,8 intraday não funciona; resultado inverte com correção de múltiplos testes | Acadêmica (refuta) |
| Osler (2000), NY Fed (https://www.newyorkfed.org/research/epr/00v06n2/0007osle.html) | Níveis de S&R "de verdade" preveem interrupções de tendência: bounce +4 a +6 p.p. acima de níveis arbitrários | Acadêmica |
| Chung & Bellotti (2021) (https://arxiv.org/pdf/2101.07410v1) | Níveis testados no passado quicam mais; efeito decai com o tempo | Acadêmica |

**Conclusão:** o valor está em **zonas com ≥2 fatores independentes** (estrutural + VWAP/POC +
Fib + âncora psicológica), nunca na razão 0,618 isolada. Nossa confluência de Fibonacci por
timeframe (1 voto por TF) está alinhada com isso — mas só como um dos votos.

## 4. GEX / gamma exposure

| Fonte | O que mostra | Evidência |
|---|---|---|
| SqueezeMetrics white paper (https://squeezemetrics.com/download/white_paper.pdf) | GEX prevê **volatilidade, não direção** (desvio 1d: 0,55% no quartil alto de GEX vs 0,85% no baixo) | White paper |
| Barbon & Buraschi, "Gamma Fragility" (https://papers.ssrn.com/sol3/papers.cfm?abstractid=3725454) | **Momentum intraday em gama negativo, reversão em gama positivo**; pico em ~60 min | Working paper (melhor evidência do tema) |
| Cboe 0DTE (2025) + tese Princeton (2026) | Fluxo 0DTE "sem impacto discernível" (0,04–0,2% da liquidez diária); canal de hedging sem poder preditivo | Institucional/acadêmica (contra) |
| Risk.net / OptionMetrics (https://www.risk.net/investing/7954930/...) | Volatilidade maior também quando o gama está **flat (zero gamma)** | Comercial séria |
| MPRA (2026) "Beyond GEX" (https://mpra.ub.uni-muenchen.de/129365) | Margem de erro 30–50% do GEX; falha em crises | Working paper (crítica) |
| GEX Metrix (https://www.gexmetrix.com/blog/oi-9am-gamma-map) | OI é publicado 1×/dia; mapa intraday é estimativa; backtest com níveis de hoje em dias passados = look-ahead | Comercial (crítica) |

**Uso defensável:** o GEX (e o nosso RegimePanel gamma POS/NEG) é **classificador de regime**
para escolher o playbook (fade vs trend). Flip > walls como referência. Nunca como suporte
garantido. Nossos níveis vêm timestamped no histórico (`<GROUP>_history.csv`), o que permite
backtest sem look-ahead — vantagem real sobre quem reconstrói GEX retroativamente.

## 5. Regime (volatilidade / trend vs range)

| Fonte | O que mostra | Evidência |
|---|---|---|
| Tese LJMU (2013) (https://researchonline.ljmu.ac.uk/id/eprint/5793/1/431316.pdf) | Filtros "no-trade" por regime de volatilidade melhoram MACD (retorno, IR, DD) | Tese |
| Regime detection em SPY (2022) (https://www.sciencedirect.com/science/article/pii/S0264999322000785) | Mudança de regime aplicada como filtro melhora mean-reversion no SPY | Acadêmica |
| MSR + KAMA/efficiency ratio (https://arxiv.org/pdf/2208.11574) | Combinação estabiliza detecção e melhora trading | Acadêmica |
| "20 regime filters" (https://setup4alpha.substack.com/p/i-tested-20-trend-based-regime-filters) | A maioria dos filtros corta trades bons e ruins na mesma proporção; bom filtro **aumenta expectancy por trade** | Análise ampla |
| Prática (nexusfi) | TREND: ADX>25, ER≥0,35, Choppiness<38; exigir 2 classificadores independentes; histerese + minimum hold | Anedótica |

**Nosso RegimePanel já faz isso** (VR de Lo-MacKinlay para persistência, percentil de RV por
horário para volatilidade) — e o nosso `research.rs::intraday` (217 regras, 0 sobreviventes)
mostra empiricamente que a maioria das regras puras não sobrevive.

## 6. VWAP de sessão

| Fonte | O que mostra | Evidência |
|---|---|---|
| Literatura de execução (Konishi 2002; Busseti & Boyd; Bialkowski et al.) | VWAP é o benchmark institucional (~50% das ordens) → fundamento de "valor justo", **sem prova de poder preditivo** | Acadêmica |
| "VWAP: The Holy Grail" (SSRN 2023) | Long acima/short abaixo do VWAP em QQQ: Sharpe 2,1 — mas período único, sem walk-forward | Auto-publicado (fraca) |
| WhiteRabbit/vwap-mean-reversion (https://github.com/WhiteRabbit-TB/vwap-mean-reversion) | Metodologia correta de validação: toques em ±2σ, bootstrap, Bonferroni, custos modelados | Referência metodológica |
| Prática ES/NQ | Primeiro pullback ao VWAP em trend day; bandas ±1σ/±2σ; evitar 11:30–13:30 ET; primeiros 15–30 min com bandas não confiáveis | Anedótica |

## 7. Gestão de saída

| Fonte | O que mostra | Evidência |
|---|---|---|
| Kaminski & Lo (2014) (https://ideas.repec.org/p/hhs/sifrwp/0063.html) | Stop-loss ajuda em momentum/regime-switching, atrapalha em mean-reversion | Acadêmica |
| Estudo sueco de trailing (https://www.quant-investing.com/blog/truths-about-stop-losses-that-nobody-wants-to-believe) | Trailing apertado (5%) é pior que buy&hold; largo (20%) foi o melhor | Acadêmica |
| Chandelier Exit (3×ATR(22)) | Stop móvel por ATR, não por % | Prática |
| Gao, Han, Li & Zhou (2018), JFE — "Market Intraday Momentum" (https://www.sciencedirect.com/science/article/abs/pii/S0304405X18301351) | Retorno da primeira meia-hora prevê o da última (R² 1,6–2,6%, OOS, **sobrevive a custos**, Sharpe 1,08); em QQQ/DIA o efeito é igual ou maior | Acadêmica forte |

**Uso:** sair/parcial na última meia-hora na direção da primeira meia-hora do dia; alvos em
níveis (não 1R arbitrário); trailing 2–3×ATR após alvo 1.

## Ranking das combinações sugeridas pela literatura

1. **Pullback em nível de confluência (POC/VWAP/S-R) com delta de confirmação, no sentido do
   Kalman/regime de tendência** — única combinação sustentada em todas as camadas.
2. **Trend following puro tipo "QTI" com regime como veto e delta como filtro** — melhor
   baseline para medir se nossos filtros adicionam algo.
3. **Fade de extremos (Edge-to-Edge / exaustão de volume) quando o regime diz lateral/gama
   positivo** — a tática de volume profile mais robusta OOS.

## O que NÃO fazer (consenso da pesquisa)

- Operar divergência de delta sem confirmação de estrutura.
- Tratar Fibonacci como sinal (refutado) — só como um voto de confluência.
- Tratar walls de GEX como suportes garantidos.
- Trailing stop percentual apertado em M1.
- Confiar em backtest de scalping sem custos (PF 1,07 sem slippage não sobrevive a 2–5 ticks).
- Esperar que o order flow preveja além de ~2 ticks de preço / poucos minutos.
- Exigir alinhamento de 3+ timeframes (nosso próprio estudo: cobertura cai de 70% para 24%).
