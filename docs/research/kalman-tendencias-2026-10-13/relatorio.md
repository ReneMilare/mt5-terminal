# Configuração do Kalman Trend para acompanhar tendências

**Recomendo inovações com ganho 2, memória 20, limite 4, janela 50, ruídos
0,02/0,002/1, entrada 1 e saída 0, no gráfico M5.** A linha usa a direção do
próprio M5; H1 aparece nas faixas, legenda e fundo como contexto. A recomendação
foi avaliada em UsaTec e UsaInd, com histórico até 08/10/2026.

O arquivo pronto é [config-recomendada.toml](config-recomendada.toml).
Para aplicá-lo, substituir somente o bloco `[studies.kalman]` no
`~/.config/mt5-terminal/config.toml`; preservar os demais blocos. O aplicativo
relê a configuração ao salvar. Usar o gráfico em **M5**: o TOML do indicador
não troca o timeframe do gráfico. O arquivo entregue passou análise TOML,
conferência dos limites reais da interface e `mt5-terminal config check` com
configuração isolada. A configuração ativa foi preservada.

## Valores e funções

| Hiperparâmetro | Valor recomendado | Papel na configuração |
|---|---:|---|
| `window` | 50 | Estimar volatilidade com os retornos anteriores; não é uma média de 50 períodos. |
| `level_noise` | 0,02 | Permitir ajustes do nível estimado sem acompanhar todo deslocamento diretamente. |
| `slope_noise` | 0,002 | Permitir mudanças de inclinação com flexibilidade moderada. |
| `measurement_noise` | 1 | Fixar a escala de observação e tornar Q/R e os limiares comparáveis. |
| `band_sigma` | 1,96 | Desenhar incerteza do nível; não participa da seleção da cor. |
| `slope_threshold` | 1 | Exigir inclinação relevante em relação à sua incerteza para iniciar a cor. |
| `slope_exit` | 0 | Manter a cor durante perda de força, abandonando-a quando a inclinação cruza zero. |
| `adaptive` | `innovation` | Adaptar os ruídos de processo pela persistência dos erros de previsão do filtro. |
| `adapt_gain` | 2 | Adaptar com intensidade moderada. |
| `adapt_span` | 20 | Dar memória exponencial à estatística das inovações. |
| `adapt_cap` | 4 | Limitar o multiplicador dos ruídos de processo ao intervalo 0,25–4. |

Os ruídos são multiplicadores de **variância**, não de desvio padrão. Os seus
rácios controlam a resposta da linha, enquanto a escala absoluta também afeta
bandas e classificação normalizada. Esse compromisso entre resposta e ruído é
consistente com a [documentação da MathWorks sobre ajuste de Kalman](https://www.mathworks.com/help/fusion/ug/tuning-kalman-filter-to-improve-state-estimation.html);
os valores específicos desta entrega vêm da implementação e avaliação locais.

As outras 14 opções, incluindo cores, bandas, espessura e confluência, estão
explicadas junto com todos os hiperparâmetros em
[hiperparametros.md](hiperparametros.md). Na entrega: `coloring="slope"`,
`confluence=["M5","H1"]`, `ribbon=true`, `legend=true`, `background=true` e
`background_alpha=22`. A lista M5/H1 não significa que ambos sejam exigidos
para colorir a linha quando `coloring="slope"`.

## O que a pesquisa encontrou

A configuração em uso tinha janela 500, eficiência com ganho 0,5/memória 200,
entrada 0,16 e saída −0,92. A janela longa suavizava a estimativa de volatilidade,
mas os Q/R maiores permitiam uma linha relativamente rápida. A saída negativa
segurava a cor mesmo quando a inclinação já apontava para o lado contrário.
Assim, a continuidade visual da cor podia esconder a perda de direção.

Avaliamos **86 configurações**, sete núcleos fixos/adaptativos, dois índices,
três blocos históricos e três visões: M5, M5+H1 e M5+M15+H1. Também avaliamos
dois cenários sintéticos com direção de drift conhecida. O motor foi o Rust
original, com histórico completo para pesquisa e parâmetros iguais entre
timeframes. A grade e critérios foram registrados em
[protocolo.md](protocolo.md) e [protocolo empírico](empirico/protocolo.md).

Comparação agregada dos dois índices em 2021–2026:

| Medida | Em uso | Recomendada | Alternativa clássica |
|---|---:|---:|---:|
| Cor contrária à própria inclinação | 23,85% | 0% | 0% |
| Tempo em estado direcional | 100% | 69,7% | 79,4% |
| Inversões de direção por sessão | 2,09 | 2,54 | 2,53 |
| Duração dos estados: média das medianas por grupo | 95 min | 49,2 min | 60,8 min |
| Distância da linha ao fechamento: média das medianas, em ATR | 0,340 | 0,345 | 0,448 |
| Cobertura correta dos deslocamentos significativos já realizados em 60 min | 83,57% | 83,11% | 86,05% |

A última medida exige deslocamento de pelo menos 1 ATR na hora **anterior**,
na mesma sessão contínua. Inclui no denominador as barras em que o filtro ficou
neutro. É um diagnóstico de acompanhamento, não previsão. O acordo com esse
movimento passado entre barras coloridas foi 99,63% na recomendada; esse valor
alto depende da definição e do mesmo histórico de preços, não representa taxa
de acerto de operações.

O ajuste recomendado preservou quase a mesma cobertura do movimento já formado
que o anterior, com neutralidade quando a direção perdeu força. Nos seis grupos
de símbolo/período, ocupação direcional ficou em 67,1–71,7%, inversões em
2,46–2,66 por sessão e distância mediana da linha em 0,332–0,356 ATR. Há mais
passagens pelo neutro que na configuração anterior: isso é parte do compromisso.

No cenário sintético de drift fraco em segmentos de duas horas, a recomendada
ficou no lado correto em 73,0% do tempo, contra 65,1% da anterior, e no lado
contrário em 14,0%, contra 34,9%. O atraso mediano desde o início do drift foi
30 minutos, contra 40. Ela teve **mais** inversões nos segmentos sem drift;
esse teste não permite afirmar superioridade em todo mercado lateral.

O ganho 4 e limite 16 aproximaram ainda mais a linha do preço, mas reduziram a
cobertura e aumentaram a quantidade de reversões confirmadas sem reconhecimento.
Não justificaram ser a escolha principal. O modo clássico com os mesmos ruídos
e limiares ofereceu maior duração/cobertura, mas a linha ficou cerca de 30% mais
afastada do fechamento que a recomendada. Ele está disponível em
[config-classica.toml](config-classica.toml), se continuidade for a prioridade.

## Como interpretar M5 e H1

Verde/vermelho da linha acompanha o estado direcional do M5; amarelo indica
que não há força suficiente para iniciar ou manter essa direção. A faixa H1
mostra a direção de uma escala maior. Fundo colorido significa acordo entre
os dois. Uma divergência pode ser um recuo ou uma mudança em formação; a
classificação, por si só, não decide qual dos dois.

Exigir o H1 para colorir a linha reduziu a ocupação de 69,7% para 29,1%.
Acrescentar M15 como outra exigência reduziu para 24,1%. Isso pode atender a
quem só quer alinhamentos mais restritos, mas deixou muito movimento M5 sem
acompanhamento. Por isso escolhi H1 como contexto. O H1 é consultado somente
quando seu candle fecha; não reage a cada tick ou a cada fechamento M5.

Os exemplos abaixo usam as sessões completas de 05, 06 e 07/10/2026, escolhidas
antes dos resultados. A linha tracejada é a configuração em uso. A faixa
“Atual” mostra sua cor M5; “M5” e “H1” mostram as cores da proposta.

![Comparação em três sessões predefinidas](figuras/comparacao-sessoes.png)

## Evidência e limites

Os históricos já foram usados em pesquisas anteriores. Os blocos temporais
demonstram repetição descritiva desse compromisso, não validação futura
independente. A direção dos 30/60/120 minutos **seguintes** continuou sem
vantagem preditiva demonstrada: os controles principais tiveram aproximadamente
51–53% de concordância condicional no horizonte de 60 minutos. Não calculamos
operações ou lucro nesta comparação.

O resultado sustenta um ajuste para acompanhar a direção em curso. Não sustenta
que entrar em cada cor gere lucro, nem que esta seja uma configuração universal
para outros ativos, timeframes ou condições de mercado. As bandas representam
incerteza do estado sob os parâmetros escolhidos, sem garantia de conter 95%
dos preços futuros.

## Reprodução e verificação

- [Relatório empírico](empirico/relatorio.md), [métricas](empirico/results/metrics.csv)
  e [agregados](empirico/results/aggregated.csv): definições, contagens, resultados e limites.
- [reproduce.sh](empirico/reproduce.sh): repetir cálculos e agregação; requer o archive
  privado já presente no projeto e dependências Rust locais.
- [plot.py](plot.py): gerar PNG/SVG das sessões; requer matplotlib.
- [dados-auditados.json](dados-auditados.json): hashes das seis séries, ordem temporal
  e consistência OHLC verificadas; 400 mil M5, 140 mil M15 e 40 mil H1 por índice.
- [config-validacao.json](config-validacao.json): TOML, limites e parser do aplicativo.
- [Fontes e pesquisas anteriores](fontes-evidencias.md): contexto bibliográfico e financeiro.

As checagens passaram nos sete núcleos e nos dois índices: cálculo incremental
igual à passada completa, classificação igual ao original, alterações futuras
sem efeito nos resultados anteriores, último candle em formação excluído e
mapeamento de timeframes maiores apenas após fechamento. As fontes numéricas
foram preservadas em `empirico/source/`, com hashes no manifesto.

O trabalho foi dividido entre dois agentes GPT-6.1 Sol com esforço alto
(matemática e avaliação empírica) e um GPT-6 Luna com esforço baixo (fontes),
seguido de revisão pelo coordenador. Opus 5.5/Haiku 5.5 não estavam disponíveis.
