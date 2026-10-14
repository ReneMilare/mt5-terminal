# Kalman Trend: parâmetros, interações e configuração inicial

Análise do código em 13/10/2026. Fonte de verdade: [trend.rs](/home/milare/Projects/mt5-terminal/preset/trend.rs:172), [math.rs](/home/milare/Projects/mt5-terminal/preset/math.rs:37), [mod.rs](/home/milare/Projects/mt5-terminal/preset/mod.rs:79) e [confluence.rs](/home/milare/Projects/mt5-terminal/preset/confluence.rs:92). Configuração em uso lida de `/home/milare/.config/mt5-terminal/config.toml`; nenhuma configuração ou implementação foi alterada.

**A configuração da seção 6 é uma hipótese anterior aos novos testes.** A entrega final,
após avaliação empírica, usa ganho 2, limite 4 e `coloring="slope"`, conforme
[relatório consolidado](relatorio.md) e [TOML recomendado](config-recomendada.toml).

O resultado principal é que **a configuração atual combina uma linha relativamente rápida com uma cor bastante persistente**. A janela 500 não transforma o indicador numa média de 500 candles. A saída negativa pode manter uma cor depois que a inclinação estimada já virou. São mecanismos distintos; reduzir trocas de cor não significa reduzir o atraso da linha nem melhorar sua capacidade de identificar tendências.

## 1. O que o indicador calcula

O estado possui nível `ℓ` e inclinação `b`, ambos no logaritmo do preço. Cada candle fechado é uma observação `y = ln(close)`. A previsão antes de incorporar esse fechamento é `ℓ⁻ = ℓ_anterior + b_anterior`, com inclinação prevista igual à anterior. A observação corrige os dois estados através dos ganhos de Kalman. A linha exibida é `exp(ℓ)`.

Isso corresponde à estrutura de tendência linear local descrita na [documentação primária do statsmodels](https://www.statsmodels.org/stable/examples/notebooks/generated/statespace_local_linear_trend.html): dois componentes de estado, observação do nível e variâncias separadas para medição, nível e tendência. O indicador local acrescenta normalização pela variância móvel dos retornos, velocidade adaptativa e classificação com histerese; esses detalhes abaixo são inferidos diretamente do código, não daquela documentação.

Defina `vᵢ` como a variância amostral dos `window` retornos logarítmicos **anteriores** ao candle `i`, e `sᵢ` como o fator adaptativo. Então:

```text
Qnível = level_noise × vᵢ × sᵢ
Qinclinação = slope_noise × vᵢ × sᵢ
R = measurement_noise × vᵢ
tᵢ = bᵢ / sqrt(Pinclinação, inclinação)
linha = exp(ℓᵢ)
banda superior = exp(ℓᵢ + band_sigma × sqrt(Pnível, nível))
banda inferior = exp(ℓᵢ − band_sigma × sqrt(Pnível, nível))
```

Os três parâmetros chamados “ruído” multiplicam **variâncias**, não desvios padrão. Dobrar `level_noise`, por exemplo, dobra a variância injetada no nível; seu desvio cresce por `sqrt(2)`. A inclinação é retorno logarítmico por candle, não pontos por minuto. O indicador não utiliza volume, máxima, mínima ou abertura para essas contas.

## 2. Todos os 11 hiperparâmetros

Os intervalos abaixo são os aceitos pelo painel/configuração, e não recomendações. Valores numéricos carregados do TOML são limitados ao intervalo; os parâmetros inteiros são convertidos para `usize` no cálculo. Valores omitidos usam os padrões.

| Chave / nome no painel | Padrão | Faixa aceita | Função e efeito de aumentar |
|---|---:|---|---|
| `window` / Janela da variância | 50 | 10–500 candles | Número de retornos anteriores usados para estimar `v`. Uma janela maior estabiliza a escala de volatilidade e reage mais devagar a mudanças de volatilidade. Não equivale a um período de média móvel e não impõe, por si, uma linha mais lenta. |
| `level_noise` / Ruído do nível | 0,02 | 0,0001–1 | Variância de movimento imprevisível do nível. Maior valor, mantendo o resto, deixa o filtro corrigir deslocamentos de preço com mais facilidade. Pode acompanhar ruído e saltos sem que a inclinação mude na mesma proporção. |
| `slope_noise` / Ruído da inclinação | 0,002 | 0,00001–1 | Variância de alteração da inclinação. Maior valor permite mudar a direção estimada mais rapidamente, com mais sensibilidade a oscilações. Valor muito pequeno preserva a direção por mais tempo e pode extrapolar além de uma reversão. |
| `measurement_noise` / Ruído da medida | 1 | 0,01–10 | Incerteza atribuída a cada fechamento. Maior valor reduz `Q/R`, geralmente confia menos no fechamento e suaviza a linha. Também muda a escala de covariância, bandas e `t`; não é somente um botão de suavização. |
| `band_sigma` / Bandas (desvios) | 1,96 | 0,1–5 | Multiplica o desvio posterior do nível para desenhar as bandas. Aumentar alarga as bandas, sem mudar a linha, inclinação, adaptação ou cores. |
| `slope_threshold` / Limiar da inclinação | 0,1 | 0–5 | Exigência de entrada em UP/DOWN, em unidades de `t`. Sem retenção anterior, UP se `t > threshold`, DOWN se `t < −threshold`; restante é FLAT. Maior limiar exige uma inclinação mais forte relativamente à incerteza e tende a reduzir classificações direcionais. |
| `slope_exit` / Saída da tendência | 0,1 | −1–5 | Exigência para manter a cor anterior. O valor efetivo é `min(slope_exit, slope_threshold)`. Reduzi-lo aumenta a persistência de cor; não modifica a linha. Zero retém UP até `t ≤ 0` e DOWN até `t ≥ 0`. Negativo aceita inclinação contrária para continuar segurando a cor. |
| `adaptive` / Velocidade | `off` | `off`, `innovation`, `efficiency` | Escolhe como `sᵢ` multiplica os dois Q. `off` deixa `s=1`. `innovation` usa persistência dos erros normalizados do filtro. `efficiency` usa eficiência direcional relativamente à sua própria referência lenta. |
| `adapt_gain` / Sensibilidade | 3 | 0–20 | Intensidade exponencial da adaptação. Maior valor amplia tanto aceleração quanto desaceleração, sujeito ao cap. Zero produz `s=1`, inclusive nos modos adaptativos. Não equivale ao ganho de Kalman `K`. |
| `adapt_span` / Memória (candles) | 20 | 3–200 | Em `innovation`, determina `α=2/(span+1)` nas médias exponenciais dos erros. Em `efficiency`, é a janela exata dos retornos do ER, e ainda determina uma referência exponencial com `α=1/(10×span)`. Aumentar torna a adaptação mais lenta. |
| `adapt_cap` / Limite da aceleração (×) | 16 | 1–100 | Limita o multiplicador de Q a `[1/cap, cap]`. Maior valor permite extremos somente se gain e estatística os alcançarem. `cap=1` desliga o efeito adaptativo. Não garante esse intervalo para ganhos K, atraso, rapidez da linha ou retornos. |

### Janela da variância: horizonte e resposta

O retorno do próprio candle `i` fica fora da variância usada na atualização dele: o código lê `returns[i-window..i]`. Assim, um salto entra primeiro como erro grande; sua contribuição à variância aparece nas barras seguintes. Uma janela curta muda o tamanho de Q e R rapidamente; a covariância P carrega memória, por isso a resposta transitória pode mudar mesmo que `Q/R` permaneça igual.

Com variância constante por tempo suficiente, a velocidade tende a depender dos rácios `level_noise/R` e `slope_noise/R`, e não do valor de `window`. Em volatilidade variável, a janela altera a evolução desses ganhos por sua interação com P. Não existe equivalência universal entre `window=50` e uma EMA/SMA de 50 períodos.

No M5, 50 candles são 4h10 de barras e 500 são 41h40. No H1, 50 candles são 50h e 500 são 500h (20d20h). Isso é tempo coberto por barras, sem converter automaticamente em dias corridos: fins de semana e sessões mudam o horizonte no calendário.

### Q do nível versus Q da inclinação

Aumentar ambos os Q na mesma proporção, mantendo R, costuma aumentar a resposta ao fechamento e às mudanças de tendência. Mas mudar apenas um deles não é equivalente: muito Q de nível deixa o nível saltar; muito Q de inclinação facilita a mudança da trajetória extrapolada. O rácio `slope_noise/level_noise` é, portanto, uma escolha distinta da velocidade geral `Q/R`.

Uma inclinação mais reativa não implica automaticamente `|t|` maior: o numerador e sua incerteza mudam juntos. Cores devem ser avaliadas depois de definir Q/R e a escala comum, e não apenas olhando a inclinação visual da linha.

### Bandas: incerteza do estado, não previsão do próximo preço

As bandas usam o P posterior do nível depois de assimilar o fechamento. Elas não adicionam R, não propagam a inclinação para o próximo candle e não calculam um intervalo para o próximo fechamento. `1,96` é um multiplicador convencional de desvios sob um modelo normal calibrado; **não demonstra cobertura de 95% dos preços de mercado**. A escala dos ruídos aqui é escolhida, a volatilidade é estimada e os preços têm saltos e caudas que não são verificados pelo indicador.

Também não são bandas de Bollinger nem canais de máximas/mínimas. A distância é simétrica no logaritmo, mas a banda superior fica mais distante em pontos que a inferior. `exp(ℓ)` representa o nível em log convertido em preço; não é a média aritmética de uma distribuição lognormal corrigida por `P/2`.

### Histerese: tabela de decisão

O código testa retenção **antes** de testar uma nova entrada. Os testes são estritos: igualdade faz a retenção/entrada daquele teste falhar.

| Cor anterior | Regra de retenção | Se ela falhar |
|---|---|---|
| UP | Continua UP enquanto `t > exit` | Testa nova UP/DOWN pelo limiar; senão FLAT. |
| DOWN | Continua DOWN enquanto `t < −exit` | Testa nova UP/DOWN pelo limiar; senão FLAT. |
| FLAT/sem estado anterior | Não há retenção | Testa UP/DOWN pelo limiar. |

Com entrada 1 e saída 0, UP entra acima de +1, segura durante a perda de força positiva e sai ao atingir zero. Se nesse candle `t=-0,3`, fica FLAT até uma eventual entrada abaixo de −1. Se `t=-1,2`, pode inverter diretamente para DOWN.

Com a configuração atual (`entrada=0,16`, `saída=-0,92`), UP continua UP em `t=-0,5`, embora a inclinação seja negativa e satisfaça a entrada DOWN. DOWN continua DOWN em `t=+0,5`, embora satisfaça a entrada UP. Na faixa `(-0,92,+0,92)`, a cor depende fortemente do caminho anterior. O limiar 0,16 só controla entrada após um estado sem retenção; as inversões de cor podem depender de ±0,92. Isso é comportamento implementado, não erro aritmético; exige interpretar a cor como estado retido, não como sinal atual da inclinação.

### Adaptação pelos erros: `innovation`

Em cada candle, a inovação é a diferença entre fechamento observado e nível previsto: `e = y − (ℓ_anterior+b_anterior)`. O filtro usa `ν=e/sqrt(S)`, com `S` a variância prevista da observação. Após atualizar o candle, calcula médias exponenciais de `ν²` e `ν×ν_anterior`. No próximo candle, utiliza:

```text
ρ = clamp(EWMA(ν×ν_anterior) / EWMA(ν²), −1, +1)
s = clamp(exp(gain×ρ), 1/cap, cap)
```

Persistência de erros do mesmo sinal sugere que a previsão está atrasada: `ρ>0` acelera. Alternância de sinais sugere oscilações: `ρ<0` desacelera. Isso é uma heurística baseada numa razão de momentos não centrados, e não uma correlação de Pearson calculada numa janela finita. Não identifica regimes de tendência com garantia.

`span=20` dá `α≈0,0952` e meia-vida de aproximadamente 6,9 candles para cada média exponencial; não significa que somente os últimos 20 candles participam. `gain=4, cap=16` alcança o teto quando `ρ≥ln(16)/4≈0,693` e o piso quando `ρ≤−0,693`. No modo off, gain, span e cap ficam sem efeito na linha.

### Adaptação pela eficiência: `efficiency`

Para os últimos `span` retornos, incluindo o candle fechado atual:

```text
ER = |soma dos retornos| / soma(|retornos|), entre 0 e 1
média_ER ← média_ER + (ER − média_ER)/(10×span)
s = clamp(exp(gain×(ER − média_ER)), 1/cap, cap)
```

Um movimento em uma direção tem ER próximo de 1; muito vai-e-volta dá ER menor. O fator usa **a diferença contra uma referência móvel**, não o ER absoluto. Um mercado persistentemente eficiente acaba elevando a referência e levando `s` novamente para perto de 1. A primeira observação válida inicializa a referência com seu próprio ER e produz `s=1`.

Com `gain=0,5`, como na configuração atual, a diferença está entre −1 e +1, logo o envelope teórico é aproximadamente `[0,607,1,649]`, normalmente ainda menor. `cap=10` nunca é o limitador. Seria preciso `gain≥ln(10)≈2,303` apenas para tornar o teto/piso de 10 teoricamente alcançável; isso não garante que os dados atinjam os extremos.

Com `span=200`, o ER cobre 16h40 de barras no M5 e 200h no H1. A referência exponencial tem constante de tempo aproximada de 2000 candles: 166h40 de barras no M5 ou 2000h no H1. É uma adaptação com memória muito longa, especialmente no H1. O trecho de histórico usado para inicializar essa referência pode influenciar resultados durante bastante tempo.

## 3. Uma interação importante: escala comum dos ruídos

Considere multiplicar `level_noise`, `slope_noise` e `measurement_noise` pelo mesmo `c>0`, mantendo os outros parâmetros e o histórico iguais. A inicialização de P também é multiplicada por c, porque usa R. Pelas equações do código, a covariância continua escalada por c, os ganhos K ficam iguais e os estados nível/inclinação ficam iguais. Em `innovation`, ambos os momentos dos erros normalizados escalam por `1/c`, portanto a razão usada na adaptação fica igual. Em `efficiency`, os retornos e ER não mudam.

Consequências, salvo diferenças numéricas e limites de validação:

- A linha central permanece igual.
- Os desvios das bandas no logaritmo são multiplicados por `sqrt(c)`.
- `slope_t` é dividido por `sqrt(c)`; a cor pode mudar mesmo com a mesma linha.
- Para preservar também a cor, os dois limiares devem ser divididos por `sqrt(c)`.
- Para preservar também as bandas, `band_sigma` deve ser dividido por `sqrt(c)`.

Isso explica por que “aumentei todos os ruídos e o indicador ficou mais lateral” pode acontecer sem suavizar a linha. Para uma pesquisa interpretável, convém fixar `measurement_noise=1`, definir os dois rácios Q/R e depois calibrar os limiares. Isso elimina uma dimensão redundante **da geometria da linha**, não uma dimensão redundante de toda a exibição.

## 4. Todas as opções de confluência e apresentação

| Chave | Padrão | Comportamento |
|---|---|---|
| `visible` | `true` | Mostra linha, bandas habilitadas e elementos associados. Oculta também ribbon, fundo e legenda; a atualização da confluência é condicionada a estar visível. Não é parâmetro de filtragem. |
| `color_up` | `#32CD32` | Cor de UP na linha/ribbon/legenda. Também fornece o RGB do fundo de alinhamento UP. |
| `color_down` | `#FF6347` | Cor de DOWN nos mesmos elementos. Também fornece o RGB do fundo DOWN. |
| `color_flat` | `#FFD700` | Cor de FLAT do próprio timeframe; usada no ribbon e na legenda. FLAT não conta como alinhamento de compra/venda. |
| `width` | 2 | Espessura da linha central, faixa 1–6. Não altera as bandas, que são desenhadas pontilhadas com largura 1. |
| `bands` | `true` | Exibe ou esconde as duas bandas. Não muda o cálculo do filtro. |
| `color_band` | `#708090` | Cor única para as duas bandas; não muda sua distância. |
| `confluence` | `["M5","M15","H1"]` | Timeframes cujos estados precisam concordar. Todos UP = alinhamento UP; todos DOWN = alinhamento DOWN. FLAT, ausência de histórico ou divergência retiram o alinhamento. A mesma configuração numérica é aplicada a todos. |
| `coloring` | `"slope"` | `slope` colore a linha pelo estado do timeframe do gráfico, inclusive sua histerese. `confluence` colore pelo alinhamento dos selecionados e usa cor neutra no restante. O nível/forma da linha continua sendo o do timeframe do gráfico. |
| `color_neutral` | `#78808C` | Cor da linha sem alinhamento quando `coloring="confluence"`. Neutro por desacordo não é o mesmo que FLAT individual. |
| `ribbon` | `true` | Uma faixa de estado por timeframe na base do gráfico, maior timeframe no topo. Permite ver quem está discordando. Não modifica os estados. |
| `background` | `false` | Fundo apenas nos candles em que todos os timeframes selecionados estão UP ou todos DOWN. Não é confirmação estatística adicional. |
| `background_alpha` | 22 | Alpha do fundo, faixa 4–120 na escala 0–255. 22 equivale a cerca de 8,6% de opacidade. Só altera aparência do fundo habilitado. |
| `legend` | `true` | Exibe estados dos últimos candles fechados de cada timeframe e a mensagem de alinhamento. “Compra/venda alinhada” é um rótulo da regra, não uma ordem nem expectativa de rentabilidade. |

O carregamento ordena e remove duplicatas de timeframes conforme a enumeração disponível. Lista vazia não produz confluência. Se o timeframe do gráfico estiver selecionado, sua linha é reutilizada; caso contrário, a confluência **não inclui automaticamente** o timeframe do gráfico. Por exemplo, gráfico M15 com seleção M5/H1 tem uma linha M15 e uma decisão de alinhamento M5/H1.

### Fechamento dos candles e atraso dos timeframes

Em cada candle histórico do gráfico, a confluência busca apenas candles de outro timeframe que já estavam fechados até o fechamento daquele candle. Um candle M5 terminando às 10h35 pode usar o H1 encerrado às 10h, mas não o H1 que encerrará às 11h. Às 11h, esse novo H1 passa a estar disponível. A condição é “fim do candle de referência ≤ fim do candle do gráfico”; não se preenche retrospectivamente os M5 anteriores com a cor H1 recém-conhecida.

Isso evita antecipação histórica no mapeamento. H1 como filtro traz atualização horária; seu estado pode ficar até quase uma hora sem uma nova observação fechada. M15+H1 adiciona outra condição e normalmente torna alinhamento mais seletivo. D1 acrescenta regime de horizonte maior, com frequência de atualização diária e possível longa ausência de alinhamento. Esses filtros não tornam a linha M5 mais rápida: restringem onde sua cor/alinhamento é mostrado ou usado.

Os mesmos 50 candles de memória têm 12 vezes mais tempo de barras em H1 que em M5. O app não oferece Q, R, window ou limiares separados por timeframe nessa configuração. É incorreto apresentar um único preset como M5 “rápido” mais H1 “lento” com calibrações independentes; o horizonte maior decorre da agregação temporal das barras.

## 5. Diagnóstico da configuração em uso

```toml
[studies.kalman]
window = 500
level_noise = 0.2805
slope_noise = 0.01502
measurement_noise = 3.26
slope_threshold = 0.16
slope_exit = -0.92
adaptive = "efficiency"
adapt_gain = 0.5
adapt_span = 200
adapt_cap = 10.0
confluence = ["M5", "H1"]
background = true
```

As demais opções omitidas usam seus padrões. Em particular, a configuração salva não contém `coloring`; portanto a linha é colorida pelo estado do próprio timeframe (`slope`). O fundo mostra confluência M5/H1, mas verde/vermelho da linha, isoladamente, não prova essa confluência.

| Aspecto | Diagnóstico |
|---|---|
| Reatividade geométrica | `Qnível/R≈0,08604` e `Qinclinação/R≈0,004607`, respectivamente 4,30 e 2,30 vezes os rácios padrão. Mesmo com R absoluto 3,26, a linha pode ser mais reativa que o padrão. |
| Distribuição entre estados | `Qslope/Qlevel≈0,05355`, contra 0,1 no padrão. Há relativamente mais flexibilidade do nível que da inclinação. |
| Janela | 500 estabiliza a variância usada como escala; não compensa os Q/R maiores como uma média de 500 faria. |
| Adaptação | Efficiency com gain 0,5 muda pouco o multiplicador e com span 200 usa referência muito lenta. O cap 10 fica inativo matematicamente. |
| Entrada | 0,16 é uma exigência baixa na escala atual; a escala R absoluta também afeta `t`. Não serve como teste de significância usual. |
| Saída | −0,92 segura o estado além da mudança do sinal da inclinação. Pode produzir aparência contínua de tendência e alinhamento mesmo durante inclinações contrárias. |
| Confluência | M5/H1 é uma estrutura coerente para comparar movimento e regime maior, mas concordância de dois estados retidos não demonstra que ambas as inclinações atuais têm o mesmo sinal. |

Como verificação algébrica independente, iterando a covariância do filtro com **variância constante igual a 1 e speed=1**, os ganhos convergem aproximadamente a:

| Configuração | Ganho do nível K0 | Ganho da inclinação K1 |
|---|---:|---:|
| Padrão Q=0,02/0,002, R=1 | 0,282 | 0,0379 |
| Atual Q=0,2805/0,01502, R=3,26 | 0,375 | 0,0536 |
| Controle lento Q=0,0006/0,00006, R=1 | 0,119 | 0,00727 |

Essa conta confirma a direção do efeito dos rácios em um caso controlado; **não mede atraso, captura ou resultado financeiro nos dados de mercado**. K0 não é um período de EMA, pois a linha inclui previsão com inclinação. Os ganhos reais variam com volatilidade, covariância e adaptação.

Outra forma de ver o problema de escala: dividir os três ruídos atuais por 3,26 produz R=1 e os mesmos níveis/inclinações, mantendo os demais parâmetros de cálculo. Para preservar as cores, entrada e saída deveriam ser multiplicadas por `sqrt(3,26)`, chegando a aproximadamente `0,289` e `−1,661`. Esta última ultrapassa o mínimo −1 aceito pelo painel. Portanto, a configuração atual usa uma combinação de escala e histerese que não pode ser convertida integralmente para R=1 dentro de todos os limites da interface. Isso não a torna numericamente inválida, mas torna menos intuitiva a comparação visual de limiares.

## 6. Configuração inicial defensável para acompanhamento

Esta é uma **hipótese inicial para comparação empírica**, não um resultado ótimo. O objetivo é uma linha que acompanhe mudanças, um limiar de entrada legível e uma cor que deixe de insistir na direção quando a inclinação cruza zero. O candidato adaptativo abaixo já existe como família nos estudos locais; a seleção final depende da avaliação dos dados e das regiões vizinhas, não somente dessa justificativa matemática.

```toml
[studies.kalman]
window = 50
level_noise = 0.02
slope_noise = 0.002
measurement_noise = 1.0
band_sigma = 1.96
slope_threshold = 1.0
slope_exit = 0.0
adaptive = "innovation"
adapt_gain = 4.0
adapt_span = 20
adapt_cap = 16.0
confluence = ["M5", "H1"]
coloring = "confluence"
ribbon = true
background = true
background_alpha = 22
legend = true
```

Para entender o filtro antes de selecionar adaptação, comparar **a mesma configuração com `adaptive="off"`**. Os dois candidatos isolam o efeito da adaptação. Um terceiro teste com M5/M15/H1 isola a seletividade adicional de M15. Um teste com D1 só faz sentido se a intenção aceitar um filtro de horizonte maior e períodos prolongados sem concordância; não deve ser acrescentado apenas por melhorar uma amostra de retorno.

O controle lento Q×0,03 (`level_noise=0,0006`, `slope_noise=0,00006`, R=1, entrada=2 e saída=2, adaptação off) também é útil para separar “menos trocas por uma linha lenta” de “menos trocas por retenção de cor”. Não é o candidato principal para quem prioriza resposta relativamente pronta a tendências intradiárias.

Fixar inicialmente window e R; comparar Q/R em uma grade pequena; depois limiar e saída; por último adaptação/timeframes evita mudar vários mecanismos ao mesmo tempo. Não perseguir o melhor ponto isolado: procurar uma região estável de parâmetros com pouco aumento de atraso para uma redução significativa das trocas. O objetivo visual pede medir atraso de entrada/saída, fração da tendência capturada, falsas inversões em lateral e tempo sem confluência. Se a cor for usada para operar, incluir spreads, slippage, exposição, drawdown, separação temporal de desenvolvimento/validação e resultado fora da amostra.

O [CLAUDE.md do preset](/home/milare/Projects/mt5-terminal/preset/CLAUDE.md:1) registra que seguir cada mudança de cor M5 perdeu para o spread nas variantes estudadas, que a adaptação não venceu consistentemente o clássico lento e que filtros de timeframes reduziram exposição/drawdown sem estabelecer vantagem robusta. A recomendação inicial acima é para **organizar acompanhamento de tendência**, não inferir uma promessa de lucro daqueles resultados.

## 7. Limites e pontos do código que importam na pesquisa

1. **Causalidade da adaptação:** innovation usa estatísticas até o candle anterior; efficiency inclui o candle fechado atual antes de atualizar seu Kalman. Os dois são causais no fechamento. O comentário de `speed_at` dizendo que usa apenas o que se sabia “before it” é exato para innovation e inexato para efficiency. Um teste que negocie na abertura do mesmo candle usando sua eficiência/estado fechado teria antecipação.
2. **Candle em formação:** `update` processa até `n−2`; o último candle fica sem novo valor do Kalman. O estado mais recente é fechado. Se o feed mantiver o último candle como “em formação” até chegar o próximo, uma barra terminal de sessão não é assimilada por esse método antes da próxima barra. Não confundir a legenda de estado fechado com reação a cada tick.
3. **Variância inválida:** o cálculo exige ao menos três retornos finitos e variância maior que `1e−18`. Retornos exatamente constantes, preços não positivos ou dados inválidos podem deixar o filtro sem valor e `ready=false`. Uma rampa determinística com retornos idênticos pode ser rejeitada apesar de uma tendência perfeita. Esse limite precisa ser respeitado em séries sintéticas.
4. **Reset parcial:** quando a variância fica inválida ou falha a atualização, o estado Kalman é marcado para reinicialização; os acumuladores de adaptação não são explicitamente zerados nesse caminho. A recuperação pode carregar memória adaptativa anterior. Mudança completa de série/parâmetros recria a estrutura e limpa essa memória.
5. **Warm-up e histórico:** a produção usa limite de 10 mil barras mais window e 256 para aquecimento, enquanto estudos podem usar modo sem limite. Efficiency com referência de 2000 candles decai devagar; 256 barras não asseguram independência da inicialização dessa referência. Histórico adicionado à frente desloca os valores existentes na linha principal; em outras rotas/passadas completas, recalcula. Comparações exigem registrar o início efetivo do histórico e o descarte de aquecimento.
6. **Covariância e robustez numérica:** o código usa atualização na forma de Joseph e verifica positividade e determinante. É uma proteção matemática útil, não validação da adequação probabilística do modelo ao mercado. Q/R precisam ser positivos; os mínimos do painel previnem zero. Estudos que instanciem `KalmanParams` diretamente precisam respeitar esses requisitos.
7. **Tempo entre candles:** a matriz de transição assume um passo por candle, sem usar o tempo corrido entre fechamentos. Uma pausa de sessão é assimilada através do próximo preço, não como vários passos de evolução do estado durante a pausa.
8. **Recalcular após editar:** mudar qualquer hiperparâmetro do Kalman provoca passada completa e reinício da confluência em thread de fundo. Até o resultado chegar, pode haver exibição transitória dos valores anteriores. Band sigma também pertence aos parâmetros de cálculo, embora matematicamente não altere nível/cores.

Esses pontos devem orientar a interpretação e os testes. Não foi feita alteração de código nesta análise; a correção de comentários ou eventuais mudanças de warm-up/reset é um trabalho separado da seleção do preset.
