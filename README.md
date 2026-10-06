# MT5 Terminal

Terminal de gráficos e ordens em Rust (egui + wgpu), rápido e leve. O MetaTrader 5 fica só como ponte
com a corretora: um Expert Advisor (`mql5/TerminalBridge.mq5`) conecta no app por TCP local e repassa
cotações, histórico, posições e ordens.

- Gráfico próprio: histórico carregado aos poucos conforme você volta no tempo, preço na frente de tudo
  (ordem das camadas configurável), teclado (←/→, +/−).
- Boleta: mercado, limite e stop, stop e alvo por distância, ZERAR, INVERTER (netting e hedge), BE.
- Operações predefinidas (ex.: 0,2 com stop de 0,20% e alvo de 0,40%): com Shift/Ctrl a ordem que
  acompanha o ponteiro já leva o volume, o stop e o alvo.
- No gráfico, como no ProfitChart: segure Shift (compra) ou Ctrl (venda) e a ordem acompanha o
  ponteiro até o clique; com Alt segurado, arrastar a linha da operação põe alvo ou stop e arrastar
  ordens/stops/alvos os move (sem Alt, arrastar move o gráfico); o × da linha (ou Delete sobre ela) fecha a posição, cancela a ordem ou tira o stop/alvo.
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
quando a pasta existe. A interface está em [src/studies.rs](src/studies.rs).

## Segurança

A ponte escuta só em `127.0.0.1`. O app não guarda credenciais: o login na corretora é feito no MT5.
Use uma conta demo para testar ordens.

## Licença

[MIT](LICENSE)
