# Protocolo descritivo registrado antes da execução

Objetivo: acompanhar movimentos com compromisso entre atraso, inversões falsas e cobertura,
sem função objetivo financeira. Dados já usados em pesquisas anteriores: blocos 2021–2022,
2023–2024 e 2025–08/out/2026. Nenhum bloco é validação intocada. Usar RTH 09:30–fechamento
NYSE do calendário original; aquecer com histórico completo anterior. Último candle em formação excluído.

Sete núcleos definidos antes dos resultados: baseline Efficiency janela500 Q=.2805/.01502 R3.26
g.5/m200/cap10; controle Off500; controle Off50 mesmos Q/R; padrão Off50 Q=.02/.002 R1;
Innovation50 Q=.2805/.01502 R3.26 g2/m20/cap4; Innovation50 padrão Q/R g4/m20/cap16;
Innovation50 padrão Q/R g2/m20/cap4. Para cada núcleo: entradas .75/1/1.5 e saídas
-.92/0/.25/.5; acrescentar baseline exato .16/-.92 e padrão exato .1/.1 (86 configurações).
Parâmetros iguais em todos os timeframes, como no aplicativo. A banda 1.96 não afeta cor ou média.

Calcular TODOS os estados usando KalmanTrend Rust original, somente com `unlimited` de pesquisa.
Reusar `slope_t` para limiares, verificando cores contra execução original em todos os núcleos,
e verificar incremental e ausência de antecipação em duas séries reais. Mapear M15 e H1 apenas
quando abertura+timeframe <= fechamento M5. Comparar M5, M5+H1 e M5+M15+H1.

Métricas: ocupação, mudanças de estado/sessão, inversões direcionais/sessão (ignorando neutro),
mediana de duração das cores dentro da sessão; fração de cor que contradiz sinal da inclinação;
erro absoluto do nível/ATR14 mediano e p90. Diagnóstico posterior em horizontes 30/60/120min,
somente dentro da mesma sessão sem buracos, e somente movimentos >=1 ATR disponível na barra:
acerto entre barras coloridas, cobertura correta entre TODOS os movimentos, cobertura ponderada
pela magnitude e cobertura oposta. Futuro é exclusivamente rótulo de avaliação, jamais sinal.
As janelas posteriores se sobrepõem: números são descritivos, sem significância ou ranking de lucro.
Complemento de interpretação após primeira execução: acrescentar acordo da cor com deslocamento
JÁ realizado nos 30/60/120min anteriores (mesma sessão, >=1ATR atual) para distinguir acompanhar
de prever. Essa nova métrica é descritiva; não muda grade nem escolha por score. Agregação de
cobertura por magnitude usa soma exata dos denominadores, não média ponderada por contagem.

Reversão real diagnosticada causalmente: deslocamento dos 12 candles anteriores >=1.5 ATR,
com sinal persistente por três candles, muda para lado oposto ao último evento confirmado,
com distância >=12 candles dentro da mesma sessão. Atraso após confirmação até três cores
consecutivas no novo lado, limitado a 12 candles. Pode ser zero se indicador já se ajustou;
isso não mede tempo desde o extremo verdadeiro. Misses reportados separadamente.

Sintético com semente fixa: ruído estacionário alternando segmentos de drift conhecido positivo,
negativo e lateral. Medir atraso desde mudança de drift até três cores corretas, limite 60 candles;
ocupação correta/contrária no drift e inversões por 100 candles laterais. Resultado sintético testa
resposta, não prova adequação aos dados reais.

Emenda de revisão antes da primeira execução: dois cenários, drift .00030 em segmentos500
(forte/longo) e drift .00010 em segmentos24 (fraco/duas horas); mesmo ruído uniforme ±.00030
mais componente alternado ±.00010, aquecimento1000. Atrasos limitados ao próprio segmento;
inversões laterais ignoram neutro, como na análise real. Eventos reais exigem janela completa
12+2 candles após confirmação, evitando contar censura no fechamento como falha.

Selecionar por equilíbrio de cobertura e inversões/atraso nos dois índices e nos três blocos;
evitar vencedor por uma métrica. Exportar sessões 05/06/07-out-2026 predefinidas completas para
comparação visual, inclusive sessões ruins. Usar visual.csv antigo só para ranking descritivo
de estabilidade e descrever divergências com objetivo de acompanhamento.
