# Kalman para acompanhar tendências: avaliação empírica

**Recomendação de compromisso:** janela 50, ruído do nível 0,02, da inclinação 0,002,
da medida 1; adaptação por inovações, ganho 2, memória 20, limite 4; entrada 1,
saída 0. Usar a cor da inclinação M5 e faixas M15/H1 como contexto. Exigir alinhamento
de todos para colorir a média corta excessivamente a cobertura de movimentos M5.
Essa escolha favorece geometria e reconhecimento da direção em curso; o clássico
com os mesmos Q/R e entrada 1/saída 0 é uma alternativa melhor para continuidade máxima.
Não há vencedor em todas as métricas e esta recomendação não implica previsão ou rentabilidade.

## Dados, grade e motor

UsaTec e UsaInd: 400.000 candles M5 e 40.000 H1 por índice; M15 do mesmo archive.
Histórico até 08/out/2026, primeira avaliação em março/2021, aquecimento anterior preservado.
RTH pela conversão servidor/New York e calendário original. Três blocos: 2021–2022,
2023–2024 e 2025–08/out/2026. **Todos esses dados já foram usados anteriormente**;
a repetição em blocos mede consistência descritiva, não generalização fora da amostra.

O protocolo registra sete núcleos × três entradas × quatro saídas, mais baseline/padrão
exatos: 86 configurações, três visões M5/M5+H1/M5+M15+H1 e horizontes 30/60/120min.
Saídas testadas: −0,92/0/0,25/0,5; entradas: 0,75/1/1,5. Bandas 1,96 mantidas;
não entram nestas métricas de média/cor. Não se calcularam operações, custos ou lucro.

**Todos os núcleos foram calculados pelo `KalmanTrend` Rust original**, usando a opção
`unlimited` já existente para estudos, sem alterar lógica de produção. As cópias de
trend/math/ctx são byte a byte iguais ao preset. Para ambos os índices e sete núcleos:
cores derivadas de `slope_t` coincidem com o original; execução incremental coincide com
passada completa; alterar preços futuros em 70% preserva todos os resultados anteriores;
último candle em formação permanece sem publicação. H1/M15 só entram quando fechados.
Mesmos parâmetros em todos os timeframes, conforme aplicativo.

## Acompanhar o movimento já formado

Há diferença entre reconhecer uma direção em curso e antecipar o próximo movimento.
Esta tabela usa deslocamento **já realizado nos 60 minutos anteriores**, magnitude ≥1 ATR14
atual e mesma sessão contínua. Acordo é entre barras coloridas; cobertura correta inclui
todas as barras com movimento significativo, incluindo aquelas que o filtro deixou neutras.
Os números agrupam os dois símbolos e os três blocos reutilizados; duração e erro são
médias de medianas por símbolo/bloco, não medianas de todas as observações reunidas.

| Perfil M5 | Acordo com movimento já formado | Cobertura correta | Ocupação | Inversões/sessão | Duração mediana média | Erro nível/ATR mediano médio |
|---|---:|---:|---:|---:|---:|---:|
| Em uso: Efficiency500, entrada0,16/saída−0,92 | 83,57% | 83,57% | 100,0% | 2,09 | 95min | 0,340 |
| Clássico padrão Q/R, entrada1/saída0 | 98,18% | 86,05% | 79,4% | 2,53 | 60,8min | 0,448 |
| Innovation padrão Q/R g2/m20/cap4, entrada1/saída0 | 99,63% | 83,11% | 69,7% | 2,54 | 49,2min | 0,345 |
| Innovation padrão Q/R g4/m20/cap16, entrada1/saída0 | 99,76% | 72,01% | 59,1% | 2,25 | 42,5min | 0,272 |

O g2 cobre quase a mesma parcela dos movimentos já formados que o baseline, com cor
mais coerente e alguma neutralidade, e mantém a linha cerca de 23% mais próxima do
fechamento que o clássico. O g4 aproxima a linha ainda mais, mas deixa mais movimentos
sem cor. Proximidade ao preço, isoladamente, também pode significar acompanhar ruído;
por isso não foi usada como critério único.

Com g2, os seis grupos símbolo/bloco apresentam ocupação 67,1–71,7%, inversões
2,46–2,66/sessão e erro de nível mediano 0,332–0,356 ATR. Com o clássico, ocupação
77,9–80,5%, inversões 2,46–2,60 e erro 0,431–0,460 ATR. O compromisso se repete nos
dois índices e nos três blocos; não resulta de uma única sessão escolhida.

## Histerese, reversões e sintético

O baseline pinta toda a RTH de verde ou vermelho. Em 23,85% das barras coloridas,
a cor contradiz o sinal da própria inclinação estimada: a saída −0,92 segura o lado antigo
mesmo após `slope_t` cruzar zero. Isso é persistência de cor, não necessariamente
persistência do movimento. Entrada1/saída0 elimina essa contradição por construção.

Elevar a saída de 0 para 0,25/0,5 geralmente reduz ocupação e duração, sem melhora material
no diagnóstico posterior. No g2 entrada1, saída0,5 cai de 69,7% para 58,9% de ocupação,
mantém praticamente as mesmas inversões e aumenta falhas de reconhecimento de reversões.
Entrada1,5 reduz ainda mais cobertura; entrada0,75 aumenta cobertura e inversões.
Entrada1/saída0 constitui um compromisso simples entre os vizinhos estudados.

A reversão real usada aqui é **confirmada após** deslocamento de 12 candles ≥1,5 ATR
e três sinais consecutivos no novo lado; depois se exige três cores consecutivas em até
12 candles. Eventos sem janela completa antes do fim da sessão são excluídos. Falhas:
baseline11,4%, clássico2,3%, g2 7,9%, g4 21,4%. Por símbolo/bloco, g2 varia de 5,7% a
10,5%. A mediana do atraso após essa confirmação é zero em vários perfis, pois o
indicador já pode ter mudado antes. **Isso não significa atraso zero desde o extremo**;
esta métrica deve ser interpretada junto com falhas e o teste controlado.

No sintético forte/longo todos os perfis cobrem ~99% do drift: é um controle fácil de
resposta, incapaz de escolher adequação intraday. O segundo sintético é mais exigente:
segmentos de duas horas, drift fraco ±0,00010 com ruído uniforme ±0,00030 e alternado
±0,00010, semente20261013, 250 transições para drift. Neutro é permitido e inversões
laterais ignoram neutro, exatamente como na métrica real.

| Sintético fraco/2h | Tempo no lado correto do drift | Tempo no lado contrário | Atraso mediano desde início do drift | Transições sem reconhecimento | Inversões/100 candles laterais |
|---|---:|---:|---:|---:|---:|
| Baseline exato | 65,1% | 34,9% | 40min | 10/250 | 0,93 |
| Clássico entrada1/saída0 | 73,3% | 18,0% | 30min | 1/250 | 2,73 |
| Innovation g2 entrada1/saída0 | 73,0% | 14,0% | 30min | 1/250 | 2,90 |
| Innovation g4 entrada1/saída0 | 70,3% | 11,8% | 30min | 2/250 | 2,07 |

O baseline consegue poucas inversões laterais conservando por mais tempo uma direção
errada. O g2 reconhece tanto drift correto quanto o clássico, reduz tempo contrário e
melhora geometria real, mas **não reduz inversões laterais neste sintético**. O g4 é
mais seletivo, com menor cobertura real. Estes cenários não modelam toda a microestrutura
dos índices e não estimam probabilidades futuras.

## Timeframes e direção posterior

No g2 entrada1/saída0, obrigar M5+H1 reduz a ocupação de 69,7% para 29,1%; acrescentar
M15 reduz para 24,1%. A falha em reversões M5 confirmadas sobe de 7,9% para 73,6% e 82,1%.
A discrepância é esperada: H1 representa uma escala maior e não precisa virar em cada
movimento intraday M5. Usá-lo como contexto mantém sua informação sem esconder a
linha de acompanhamento M5. Se o usuário quiser apenas tendências de escala maior,
a cor por confluência é uma escolha distinta, com essa perda explícita de cobertura.

O diagnóstico posterior avalia deslocamentos dos 30/60/120min seguintes ≥1 ATR disponível
na barra, sem atravessar sessão ou buracos. No horizonte60min, acerto direcional entre
barras coloridas fica aproximadamente 51–53% para os controles principais, incluindo
confluência. A cobertura correta posterior é 51,3% baseline, 41,0% clássico, 36,6% g2 e
31,2% g4: maior cobertura do baseline decorre em parte de nunca ficar neutro. Esse
resultado **não demonstra vantagem preditiva**; o ganho defendido é descrição coerente
do movimento em curso. Janelas sobrepostas e dois índices correlacionados impedem tratar
estas contagens como observações independentes para inferência simples.

## Relação com pesquisa visual anterior e limites

Reutilizou-se `visual.csv` anterior somente para ordenar descritivamente mudanças/sessão,
ocupação e alinhamento. Os perfis mais estáveis desse ranking pintam frações próximas
de zero: o primeiro tem 0,0046 mudanças/sessão e ocupação0,056%. Escolher só estabilidade
produziria um indicador quase sempre neutro. Os resultados anteriores voltados a lucro
também não foram usados como função objetivo deste diagnóstico.

Efficiency500 ganho0,5 tem velocidade mediana média0,994, p10≈0,964 e p90≈1,052;
com os mesmos limiares seus controles Off500 são praticamente idênticos. Já Innovation g2
tem mediana≈3,17 e p90 no cap4; g4 mediana≈7,16 e p90≈13,85. A adaptação é substancial
e pode estar frequentemente limitada: isso reforça a preferência pelo controle moderado,
sem alegar que o ganho2 seja um ótimo único.

Métrica de nível compara filtro causal com fechamento observado; não mede erro contra
uma tendência latente verdadeira. O acordo retrospectivo pode ser alto porque ambos
derivam do mesmo histórico de preços. Reversões reais confirmadas são apenas um proxy;
flips, recuos e movimentos futuros não definem universalmente uma inversão falsa.
As sessões05/06/07-out-2026 foram definidas antes da execução e exportadas inteiras para
inspeção; servem como exemplos de forma, não evidência independente.

## Reprodução e arquivos

Executar `bash reproduce.sh` neste diretório; requer Rust, Python3 e dependências Rust
já disponíveis para `--offline`. `KALMAN_DATA=/tmp/kalman-trend-data bash reproduce.sh`
reaproveita os dados locais. Sem essa variável, o script extrai M5/H1/M15 do archive
existente para `data/` (ignorado pelo git). `target/` também é ignorado.

- `protocolo.md`: grade e métricas; emendas de revisão identificadas.
- `diagnostics.rs`, `source/`, `Cargo.toml` e `Cargo.lock`: motor original e diagnóstico reproduzível.
- `analyze.py`: agregação descritiva, sem score financeiro ou vencedor automático.
- `results/metrics.csv`: 4.644 linhas por configuração/símbolo/bloco/visão/horizonte.
- `results/aggregated.csv`: contagens agregadas exatas e médias de medianas identificadas.
- `results/synthetic.csv`: 172 linhas para dois sintéticos.
- `results/parameters.csv`: 86 configurações; `existing_visual_descriptive.csv`: ranking visual antigo.
- `results/series_*`: OHLC, ATR, linha, inclinação, velocidade e cores nas três sessões;
  sufixo `baseline_exact` usa0,16/−0,92, demais sufixos usam entrada1/saída0.
- `results/run.log`: verificações originais, incrementais, causais e alinhamento temporal.
- `manifest.json`: hashes e descrição dos dados/código.

Nenhuma alteração no preset, no código de produção ou na configuração do terminal foi feita
por esta avaliação. A recomendação é concretizada no relatório para revisão/aplicação do agente principal.
