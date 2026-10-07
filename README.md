# MT5 Terminal

Terminal de gráficos e ordens em Rust (egui + wgpu), rápido e leve. O MetaTrader 5 fica só como ponte
com a corretora: um Expert Advisor (`mql5/TerminalBridge.mq5`) conecta no app por TCP local e repassa
cotações, histórico, posições e ordens.

- Gráfico próprio: histórico carregado aos poucos conforme você volta no tempo, preço na frente de tudo
  (ordem das camadas configurável), teclado (←/→, +/−).
- Bid e ask com linhas e preços identificados no gráfico; spread em preço na legenda, atualizado a cada tick.
- Modos Seta, Mão (arrastar o gráfico) e Cruz (clicar e arrastar para medir variação em % e distância em barras;
  solte para manter a medição, Esc para limpar). O cursor escolhido fica salvo; os três modos usam ícones na barra.
- Boleta: mercado, limite e stop, stop e alvo por distância, ZERAR, INVERTER (netting e hedge), BE.
- Operações predefinidas (ex.: 0,2 com stop de 0,20% e alvo de 0,40%): com Shift/Ctrl a ordem que
  acompanha o ponteiro já leva o volume, o stop e o alvo.
- No gráfico, como no ProfitChart: segure Shift (compra) ou Ctrl (venda) e a ordem acompanha o
  ponteiro até o clique; com Alt segurado, arrastar a linha da operação põe alvo ou stop e arrastar
  ordens/stops/alvos os move (sem Alt, use Mão para mover o gráfico e Cruz para medir); o × da linha nos modos
  Seta/Mão (ou Delete sobre ela) fecha a posição, cancela a ordem ou tira o stop/alvo.
- Troca entre conta demo e real pela barra de status (o app reinicia o MT5 na conta escolhida; a
  senha fica só no MT5).
- Conta REAL travada até você armar a boleta; fonte **Sintético** com corretora simulada para testar
  sem o MT5.

## Uso

Requer Rust (edition 2024) e o MetaTrader 5 (no Linux, via Wine).

```sh
cargo build --release
target/release/mt5-terminal              # abre o MT5 se ele não estiver aberto
target/release/mt5-terminal --synthetic  # sem MT5, dados e corretora simulados
```

No MT5:

1. Compile e instale o EA: `tools/build-ea.sh` (ou abra `mql5/TerminalBridge.mq5` no MetaEditor).
2. **Ferramentas → Opções → Expert Advisors**: permita WebRequest para `127.0.0.1`.
3. Anexe `MT5Terminal/TerminalBridge` a um gráfico e ligue o **Algo Trading**.

Protocolo e detalhes: [docs/protocol.md](docs/protocol.md).

## Configuração

`~/.config/mt5-terminal/config.toml`, comentado e aplicado na hora ao salvar (`mt5-terminal config
init` cria, `config check` valida). Com o app aberto, `mt5-terminal ctl help` lista comandos como
`ctl symbol UsaInd`, `ctl tf H1`, `ctl front price` e `ctl state`. Agentes (Claude Code) têm uma skill
em `.claude/skills/configure-mt5-terminal`. Nada disso envia ordens.

## Indicadores

O app não traz indicadores próprios: um preset opcional em `preset/` (outro repositório) é compilado
quando a pasta existe. O preset inclui Fibonacci automático no **M5, M15, H1 e D1**, integrado
às convergências do mapa de suportes e resistências. Usa o último movimento entre topo e fundo
confirmados por 2 candles fechados de cada lado, dentro dos últimos 300 candles fechados de cada
série. Retrações padrão: **23,6%, 38,2%, 50%, 61,8% e 78,6%**. Cada etiqueta identifica timeframe e
percentual, alinhada à extrema esquerda do gráfico; ao passar o mouse, aparecem todas as referências
com seus preços. Vários percentuais
do mesmo Fibonacci contam como uma referência na convergência; timeframes diferentes contam
separadamente. O mapa mantém até três níveis de cada lado do preço, priorizando convergências.
Sem um par de pivôs confirmado, aquela série aguarda confirmação. Ajustes em `[fibonacci]` no
config.toml: `enabled`, `lookback` (20–2000), `pivot_bars` (1–10) e `levels` (frações de 0 a 1).

A interface está em [src/studies.rs](src/studies.rs).

## Segurança

A ponte escuta só em `127.0.0.1`. O app não guarda credenciais: o login na corretora é feito no MT5.
Use uma conta demo para testar ordens.

## Licença

[MIT](LICENSE)
