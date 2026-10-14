# Estratégia de daytrade sobre os indicadores do preset — proposta e plano de validação (2026-10-13)

Pergunta: com os indicadores que já construímos (KalmanTrend + confluência, delta/POC,
LevelMap com Fibonacci multi-TF, RegimePanel persistência/vol/gama, SessionVWAP, opções IV/skew,
força relativa), que estratégia de daytrade é lucrativa?

Resposta curta e honesta: **nenhuma regra pura dos nossos indicadores sobreviveu ao nosso
próprio backtest até hoje** (`preset/research.rs::intraday`: 217 regras, 0 sobreviventes no
critério t≥2 diário; o estudo do Kalman mostra acerto direcional de 51–53% em 60 min). A
literatura converge para o mesmo lugar: o edge não está em nenhum indicador isolado, está na
**hierarquia** — regime decide o playbook, o nível dá a zona, o flow dá o timing, a gestão
protege. A proposta abaixo é o candidato **nº 1 dessa hierarquia**, montado exatamente com as
peças que já temos, seguido de um plano de falsificação no nosso próprio harness.

Detalhe das fontes: [fontes-evidencias.md](fontes-evidencias.md).

## 1. O que cada indicador pode (e não pode) fazer

| Indicador | Papel legítimo | Papel que a evidência **não** sustenta |
|---|---|---|
| RegimePanel (persistência VR, vol por percentil, gama) | Escolher o playbook e dimensionar | Prever direção |
| KalmanTrend M5 + confluência M15/H1 | Definir a **única direção permitida** | Ser gatilho de entrada ("seguir a cor perde para o spread") |
| LevelMap (score ≥ 2: Fib por TF, POC NY-1, VAH/VAL, VWAP D/S/M, OR 15m NY, MMs, walls GEX, máx/mín D-1) | Zona de entrada/alvo — confluência de ≥2 fatores independentes | Fibonacci isolado como sinal (refutado por Tsinaslanidis et al. 2022) |
| VolumeDelta + POC (filtro seletivo z∈[1,2]) | **Timing** de curto prazo (o alfa do flow decai em ~2 ticks — Kolm et al.) | Prever swing; delta sozinho é estimativa ruidosa (tick rule, 66–93% de acerto) |
| SessionVWAP | Valor justo da sessão, alvo de reversão, filtro direcional | Poder preditivo por si só |
| Opções IV/skew + faixas 1σ | Dimensionar alvos ("o dia já andou Xσ?") e evitar entradas tardias | Sinal direcional |
| Força relativa (UsaTec/Ind/Rus) | Escolher o símbolo mais forte/menos fraco do dia | — |

## 2. Estratégia proposta — "Pullback de Confluência" (playbook principal)

Símbolo/timeframe: UsaTec ou UsaInd no **M5**, gatilho em **M1/M5**. Meta: **2–4 operações/dia**.

### A. Contexto — só opera se:
1. **RegimePanel persistência = TREND_UP** (compra) ou **TREND_DOWN** (venda). `NEUTRAL`/`REVERT`
   → playbook B (seção 3) ou nada feito.
2. **KalmanTrend M5** aponta o mesmo lado (shade UP/DOWN, `slope_t` acima do limiar de entrada —
   `kalman.adaptive="innovation", adapt_gain=2, adapt_span=20, adapt_cap=4, slope_threshold=1,
   slope_exit=0`, conforme `docs/research/kalman-tendencias-.../relatorio.md`). **Não** exigir
   alinhamento total M5+M15+H1 (corta a cobertura para 24%); usar M15/H1 só como contexto na
   legenda/ribbon — se M15 e H1 apontarem contra, reduzir o tamanho pela metade.
3. **Volatilidade ≠ HIGH** no percentil por horário (p<80); vol ALTA só permite o playbook B.
4. Nada de entrada nos **primeiros 15 min** após a abertura NY, entre **11:30–13:30 ET**, nem
   **±15 min de dado macro** (8:30/10:00 ET).

### B. Zona de entrada:
5. Preço em pullback até um **nível do LevelMap com score ≥ 2 do lado da operação** (ex.: VWAP D
   + Fib 61,8% do M15; POC NY-1 + VAH/VAL; OR 15m NY + MM20 H1). Zona = nível ± metade da
   tolerância do próprio mapa (ATR diário × 0,04).
6. Se o nível coincide com o **POC de uma barra recente** (marca dourada) ou com a **borda do
   cluster de volume**, melhor.

### C. Gatilho (fechamento de barra, nunca por tick):
7. Barra que **rejeita a zona** (ex.: compra: close acima do máximo da barra anterior após tocar
   a zona) **com delta no mesmo lado** (delta > 0) e **volume ≥ 1,2× a média das últimas 20
   barras** (o achado do estudo honesto de volume profile: PF 1,14 → 1,63–1,89 com barra de
   sinal de volume).
8. Reforços (opcionais, nunca gatilhos isolados): divergência de delta (fundo novo com delta
   positivo) e/ou POC seletiva (`delta_filter=true`, z∈[1,2]) marcada na barra do fundo.

### D. Gestão:
9. **Stop:** do outro lado da zona + buffer (½×ATR(12) do M5), ou abaixo do último swing
   confirmado. Risco fixo por operação (ex.: 0,25–0,5% da conta).
10. **Alvo 1 (50%):** próximo nível do LevelMap do lado oposto ou VWAP D. **Runner:** Chandelier
    **2,5×ATR(22)** (trailing por ATR, nunca % apertada).
11. **Saída por tempo:** sem continuação em 3 barras fechadas → sai. **Fechamento:** sai (ou
    protege) na **última meia-hora** do pregão, preferencialmente na direção da primeira
    meia-hora do dia (Gao et al. 2018, JFE — sobrevive a custos).
12. **Teto de operações:** 2 falhas no mesmo nível = para no dia. Máx. 4 operações/dia.

### Leitura de gama (RegimePanel):
- **Gama negativo** (momentum intraday, Barbon & Buraschi): permite segurar o runner; stop
  mais largo; **sem operar contra o movimento**.
- **Gama positivo** (reversão): alvos mais curtos (VWAP como alvo final); considerar direto o
  playbook B.

## 3. Playbook B — "Fade de Extremos" (regime REVERT/range, gama positivo)

Só quando: persistência = REVERT (ou NEUTRAL com vol CALM/NORMAL) **e** gama positivo **e**
sem evento macro.

1. Preço estica até confluência de **≥2 fatores no extremo**: banda/extremo do dia (σ do IV ou
  distância em ATR), wall de gama ou VAH/VAL, Fib 61,8/78,6.
2. Confirmação: rejeição com 3 barras fechadas voltando para dentro + delta virando no sentido
  do fade (exaustão: mínima nova com volume fino — a tática de volume profile mais robusta OOS).
3. Stop: além do extremo + ½×ATR. **Alvo: VWAP D** (não a média do Kalman). Sem trailing.
4. Este playbook tem payoff curto → exige win rate alto; teste antes com os controles
  adversariais (níveis aleatórios).

## 4. O que propositalmente ficou de fora

- Fibonacci como regra (refutado) — entra só como 1 voto do LevelMap.
- Exigir confluência M5+M15+H1 completa (nosso estudo: cobertura 70% → 24%).
- Seguir a cor do Kalman como entrada (líquido negativo em todas as variantes).
- GEX walls como suporte garantido (erro de 30–50%, OI defasado) — só como contexto de regime
  e, quando usados como nível, exigindo os mesmos ≥2 votos do mapa.
- Qualquer regra de "golden pocket" 38,2–61,8 (refutada em FX intraday, 2026).

## 5. Plano de falsificação (usar o harness que já existe)

Métrica de sucesso (mesma de `research.rs::intraday`): **t diário ≥ 2, média > 0 em ≥2 símbolos,
> 0 em ≥3 de 4 blocos, ≥ 60 operações**, e só então olhar a reserva 2021-03..2022-12
(`RESEARCH_HOLDOUT=1`).

1. **Codificar os playbooks A e B como Família P/Q em `preset/research.rs`** (mesma execução das
   regras existentes: sinal no fechamento, fill na abertura seguinte, stop/alvo intrabar
   conservador, custos 0,8/0,7/1,25 bp).
2. **Variações mínimas e predefinidas** (evitar grid search): gatilho {rejeição 1 barra, rejeição
   3 barras} × volume {sem filtro, ≥1,2×} × stop {zona+½ATR, 1,5 ATR} × alvo {nível, 2R} = 16
   regras por playbook.
3. **Controles adversariais** (obrigatórios): (a) mesmas entradas em **níveis aleatórios**
   equivalentes; (b) permutação do volume; (c) sem o delta (ablação); (d) semente aleatória
   como o `Z controle` existente. Se o setup não vencer os controles, ele é decoração.
4. **Segmentação:** por regime (TREND vs REVERT, gama POS/NEG), por hora (abertura vs 11:30–13:30
   vs última meia-hora) e por símbolo.
5. **Sensibilidade:** variar limiares ±20% e exigir degradação suave (superfície plana).
6. Se algo sobreviver: **4–6 semanas de paper/demo com parâmetros congelados** (o terminal já
   opera em demo) antes de qualquer dinheiro real.

Comandos:

```bash
# dados: dump-history.py (M5:80000 M15:30000 H1:10000) com o app fechado
MT5_STUDY_DIR=DIR cargo test --release -- --ignored --nocapture --exact preset::research::intraday
# depois, com os finalistas:
RESEARCH_HOLDOUT=1 RESEARCH_FINALISTS="familia_p_01;familia_p_02" MT5_STUDY_DIR=DIR cargo test --release -- --ignored --nocapture --exact preset::research::intraday
```

## 6. Expectativas realistas

- Win rate esperado do playbook A: 45–55% com payoff estrutural (alvo em nível > stop de zona).
- A literatura diz que poucos (ou nenhum) setups passam em walk-forward com custos — o nosso
  próprio `intraday` encontrou **0 em 217**. O valor desta proposta é ser **falsificável**:
  16 regras + 4 controles, critério definido antes de rodar.
- Se a Família P também morrer no holdout, o resultado legítimo é: **day trade direcional em
  CFD de índice não tem edge com estes indicadores** — e aí a conclusão correta é operar menos
  (o estudo do Kalman mostra que "operar menos" é o que melhora) ou mudar de horizonte
  (`research.rs::daily`: "Média 200 no D1 só reduz drawdown").
