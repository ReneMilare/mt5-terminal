#!/usr/bin/env bash
# Compila o EA TerminalBridge com o MetaEditor do MT5 numa pasta temporária e, só se passar sem erros
# nem avisos, instala em MQL5/Experts/MT5Terminal. Usa o mesmo Wine que serve o prefixo (o do MT5
# aberto): nunca dois Wines no mesmo prefixo ao mesmo tempo.
set -euo pipefail

WINEPREFIX="${WINEPREFIX:-$HOME/.mt5}"
MT5_DIR="${MT5_DIR:-$WINEPREFIX/drive_c/Program Files/MetaTrader 5}"
if [ -z "${WINE:-}" ]; then
    for p in $(pgrep -x wineserver); do
        if tr '\0' '\n' < "/proc/$p/environ" 2>/dev/null | grep -qx "WINEPREFIX=$WINEPREFIX"; then
            WINE="$(dirname "$(readlink "/proc/$p/exe")")/wine"
            break
        fi
    done
fi
# sem MT5 aberto: o Wine do atalho do MetaTrader 5
WINE="${WINE:-wine}"
echo "Wine: $WINE"

root="$(cd "$(dirname "$0")/.." && pwd)"
stage="$(mktemp -d -t mt5-terminal-ea.XXXXXX)"
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/MQL5/Experts"
cp -r "$MT5_DIR/MQL5/Include" "$stage/MQL5/"
cp "$root/mql5/TerminalBridge.mq5" "$stage/MQL5/Experts/"

# caminho Z: sem espaços (o MetaEditor não aceita /compile com espaço no caminho)
win() { printf 'Z:%s' "${1//\//\\}"; }
WINEPREFIX="$WINEPREFIX" WINEDEBUG=-all timeout 120 "$WINE" "$MT5_DIR/MetaEditor64.exe" \
    "/compile:$(win "$stage/MQL5/Experts/TerminalBridge.mq5")" "/inc:$(win "$stage/MQL5")" \
    "/log:$(win "$stage/build.log")" >/dev/null 2>&1 || true

text="$(iconv -f UTF-16LE -t UTF-8 "$stage/build.log" 2>/dev/null | tr -d '\r' || true)"
echo "$text" | grep -E "error|warning|Result" || echo "${text:-log ausente}"
if [ ! -f "$stage/MQL5/Experts/TerminalBridge.ex5" ] || ! grep -q "0 errors, 0 warnings" <<<"$text"; then
    echo "compilação não passou: nada instalado" >&2
    exit 1
fi

dest="$MT5_DIR/MQL5/Experts/MT5Terminal"
mkdir -p "$dest"
cp "$stage/MQL5/Experts/TerminalBridge.mq5" "$stage/MQL5/Experts/TerminalBridge.ex5" "$dest/"
echo "instalado em $dest (no Navegador do MT5: Expert Advisors > clique direito > Atualizar)"
