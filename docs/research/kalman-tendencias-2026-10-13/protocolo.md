# Pesquisa de acompanhamento de tendências — 13/10/2026

Pedido: compreender todas as entradas do Kalman Trend, coordenar agentes e entregar
uma configuração para acompanhar tendências. O usuário autorizou explorar combinações
de timeframes. A base disponível é o histórico real local de UsaTec e UsaInd,
fevereiro/2021 a 08/10/2026. A pesquisa usa o modelo efetivamente implementado em
`preset/trend.rs` e `preset/math.rs`.

## Divisão do trabalho

- GPT-6.1 Sol, esforço alto: análise matemática, interações e documentação das entradas.
- GPT-6.1 Sol, esforço alto: avaliação empírica do acompanhamento de tendências.
- GPT-6 Luna, esforço baixo: levantamento de fontes primárias e evidências anteriores.
- Agente coordenador: revisão das conclusões, fontes, arquivos de configuração e exemplos.

Opus 5.5 e Haiku 5.5 não estão disponíveis nesta sessão. Nenhuma chamada a esses
modelos foi realizada.

## Critérios definidos antes dos novos resultados

O objetivo principal é acompanhar movimentos direcionais com persistência razoável,
sem esconder uma reversão durante muito tempo. Lucro de uma regra operacional é
outra avaliação: esta pesquisa não seleciona uma estratégia pelo maior resultado
financeiro de milhares de tentativas.

Comparar a configuração em uso, o padrão do indicador e controles fixos/adaptativos.
Grade proposta pelo agente empírico antes de observar seus resultados: sete núcleos
(configuração em uso; mesma configuração sem adaptação; mesma configuração fixa
com janela 50; padrão; inovações com os ruídos em uso e ganho 2/memória 20/limite 4;
inovações com os ruídos padrão e ganho 4/memória 20/limite 16;
inovações com os ruídos padrão e ganho 2/memória 20/limite 4).
Entradas 0,75/1/1,5 e saídas −0,92/0/0,25/0,5, respeitando saída ≤ entrada;
mais os parâmetros exatos em uso e padrão. Bandas não participam da seleção.

Avaliar mudanças de estado por sessão, ocupação direcional, duração dos estados,
erro da linha normalizado pelo ATR, concordância com movimentos posteriores de
30/60/120 minutos e cobertura dos movimentos de pelo menos 1 ATR. O preço posterior
é usado somente como diagnóstico, nunca para formar o sinal. Avaliar também
resposta em trajetórias sintéticas com tendência conhecida. Para combinações de
timeframes, consultar apenas candles maiores já fechados no instante observado.

Separar resultados de 2021–2022, 2023–2024 e 2025–2026. Esses históricos já foram
examinados em pesquisas anteriores: a separação temporal não os transforma em
validação independente intocada. Barras sobrepostas, sessões e índices correlacionados
não devem ser tratados como observações independentes para alegar significância.

## Entrega

Documentação de todas as entradas, métricas e scripts reprodutíveis, relatório
consolidado e TOML compatível com as opções reais do aplicativo. A configuração
recomendada deve expressar o compromisso observado, com seu horizonte e limites.
Alterações preexistentes no código, pesquisas anteriores e a configuração ativa
do usuário serão preservadas.
