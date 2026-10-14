# POC por força do delta: teste visual

Hipótese solicitada em 13/10/2026, implementada e ativada no MT5 Terminal.
O usuário confirmou o uso da **magnitude do delta, |delta|, nos dois lados**.
O gráfico ativo na comparação era **UsaRus M5**.

## Regra aplicada

Para cada candle fechado, usar `x = |compra − venda|` e calcular a média e o
desvio padrão amostral dos **20 candles anteriores**, sem incluir o candle
classificado. A POC já existente aparece somente quando:

```text
z = (|delta do candle| − média anterior) / desvio anterior
1 ≤ z ≤ 2
```

Compra e venda usam a mesma referência de magnitudes. O candle em formação
fica sem marca neste modo. Dados ausentes ou desvio zero não geram sinal;
não são substituídos por zero. Candles acima de 2 desvios ficam de fora,
conforme o intervalo escolhido. O desvio amostral usa divisor 19.

A POC continua sendo o nível calculado pelos ticks do candle; o filtro apenas
seleciona sua exibição. O delta continua sendo o delta pela regra de ticks já
implementada no terminal. Nenhuma alteração foi feita no cálculo bruto da POC,
do delta ou do Kalman.

## Resultado na janela real

Na captura final havia **3.000 POCs fechadas** disponíveis, das quais **2.980**
tinham a referência completa de 20 candles. O filtro selecionou **310**:
aproximadamente **10,4%** das POCs com referência válida. Estes números se
referem ao histórico carregado, não somente aos candles visíveis ou ao dia.
As marcas selecionadas continuam sendo os pequenos traços amarelos nos candles.

| Exibição | Captura |
|---|---|
| Todas as POCs | [Antes](antes.png) |
| POCs pela magnitude do delta entre 1σ e 2σ | [Depois](depois.png) |

![POCs filtradas no UsaRus M5](depois.png)

As duas capturas foram feitas na mesma instância atualizada, com o mesmo
enquadramento do gráfico. Ticks recebidos entre as capturas mudam a cotação e
o candle em formação. Metadados da aplicação estão em `antes.json` e
`depois.json`; a captura inicial anterior à atualização foi preservada
separadamente.

Este teste verifica seleção, densidade das marcas e apresentação no gráfico.
Não mede ainda se o preço reage mais nessas POCs, nem se elas oferecem vantagem
operacional. Essa avaliação exigiria uma regra de reação e um estudo posterior
com as POCs e os deltas reais, respeitando o instante em que cada marca ficou
disponível.

## Configuração e uso

O [bloco de teste](config-teste.toml) foi aplicado a
`~/.config/mt5-terminal/config.toml`, preservando os demais blocos e os presets.
Foi salvo backup com o sufixo `.bak-poc-delta-20261013-205349`.

Na janela **Editar POC por candle**, estão disponíveis:

- **Filtrar pela força do delta:** ativar ou voltar a mostrar todas as POCs.
- **Referência (candles anteriores):** 20 neste teste.
- **Mínimo acima da média (desvios):** 1 neste teste.
- **Máximo acima da média (desvios):** 2 neste teste.

A configuração padrão do código deixa o filtro desligado para preservar a
exibição anterior em instalações sem este bloco. Ele está **ligado** na
configuração ativa do usuário. `ctl state.poc` permite conferir a seleção sem
depender de contagem visual dos pontos.

## Implementação e validação

O cálculo usa Welford para a variância. Mudanças apenas no candle em formação
não recalculam os resultados fechados; lotes de delta fechado, novo histórico
ou parâmetros alterados invalidam o cache. Correções do histórico podem
atualizar marcas e suas janelas de referência. O cálculo novo cobre somente o
segmento limitado de deltas já usado pelo indicador.

Passaram cinco testes específicos: referência anterior de 20 candles e
simetria de sinal, limites inclusivos e exclusão acima de 2σ, referência ausente
ou constante, ticks/fechamentos/correções/histórico anteposto e integração com
a exibição preservando a POC bruta. A suíte com preset passou **112 testes**
(11 estudos/benchmarks ignorados); a cópia isolada sem preset passou **70**.
`cargo clippy --release` passou sem avisos e os TOMLs foram validados.

Os testes gerais usam perfil temporário, pois um teste de controle grava tema;
o teste da ponte usa um socket local efêmero. A verificação isolada sem preset
interferiu inicialmente no cache de compilação do aplicativo; sua identidade
foi separada e a compilação com preset foi refeita. A presença das novas opções,
a conexão real e as contagens foram confirmadas na janela final.

Arquivos principais: `preset/poc.rs`, integração em `preset/mod.rs`, exposição
do estado em `src/main.rs` e suporte sem preset em `src/studies.rs`.
Resultados dos testes estão em [testes.txt](testes.txt).
