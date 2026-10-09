//+------------------------------------------------------------------+
//| TerminalBridge.mq5                                               |
//| Ponte do MT5 Terminal (Rust): o MT5 só conecta na corretora.     |
//| Cliente TCP do app (sockets do MQL5 só conectam, não escutam).   |
//| Protocolo: um objeto JSON por linha, campo "t" = tipo.           |
//| Veja docs/protocol.md.                                           |
//+------------------------------------------------------------------+
#property copyright "MT5 Terminal"
#property version   "8.00"
#property description "Ponte do MT5 Terminal: cotações, histórico e ordens por TCP local."

input string InpHost      = "127.0.0.1"; // Endereço do app
input ushort InpPort      = 47011;       // Porta do app
input ulong  InpMagic     = 47011;       // Número mágico das ordens do app
input ulong  InpDeviation = 20;          // Desvio máximo a mercado (pontos)
input int    InpTimerMs   = 10;          // Ciclo (ms): ticks, comandos e estado

#define BRIDGE_VERSION 9 // sobe quando o protocolo muda; o app avisa se o EA for mais antigo
#define RECONNECT_MS 250 // no localhost, tentar sem app falha na hora: reconectar logo abre o app mais rápido
#define STATE_MS     100
#define SYMBOL_MS    5000 // valores monetários do tick acompanham conversões cambiais
#define MAX_TICKS    2000

int      g_sock = INVALID_HANDLE;
uchar    g_rx[];
ulong    g_nextConnect = 0;
bool     g_hintShown = false;

string   g_syms[];       // símbolos assinados
long     g_lastMsc[];    // último tick enviado de cada um

// requisições assíncronas em voo: request_id do MT5 -> id do app
uint     g_reqIds[];
long     g_cliIds[];

// pedidos de histórico; os que ainda carregam no terminal são tentados de novo
string   g_hSym[];
string   g_hTf[];
int      g_hCount[];
long     g_hBefore[];   // 0 = os mais recentes; senão, os anteriores a este horário
int      g_hTries[];

ulong    g_nextState = 0;
ulong    g_nextSymbol = 0;

// delta de volume por candle (comando "delta"): um pedido por vez, uma leitura de ticks por ciclo
#define DELTA_CHUNK_MSC 86400000 // ticks lidos em blocos de até 24 horas: cada leitura custa ~120 ms fixos no MT5, quase independente do tamanho; menos leituras = menos pausas para ticks e ordens
#define DELTA_TRIES     20       // bloco sem ticks (histórico baixando) é pulado depois disso
string   g_qSym[];
string   g_qTf[];
int      g_qCount[];
int      g_qSkip[];                // candles fechados mais novos a pular (o app já os pediu antes)
double   g_qRow[];
double   g_dRow = 0;              // altura do nível da POC (0 = sem POC)
string   g_dSym = "";
string   g_dTf = "";
long     g_dPeriodMsc = 0;
datetime g_dTimes[];             // candles fechados do pedido, em ordem
int      g_dNext = -1;           // próximo candle a calcular (do mais novo ao mais antigo)
int      g_dTries = 0;
string   g_lastPositions = "";
string   g_lastOrders = "";
string   g_lastAccount = "";
string   g_lastDaily = "";
datetime g_dayStart = 0;
double   g_dayRealized = 0.0;
bool     g_dayDirty = true;
bool     g_dayReady = false;
ulong    g_nextDayRetry = 0;

//+------------------------------------------------------------------+
//| JSON mínimo: só objetos planos que o app envia                   |
//+------------------------------------------------------------------+
string JStr(const string j, const string key)
  {
   string pat = "\"" + key + "\":\"";
   int p = StringFind(j, pat);
   if(p < 0)
      return("");
   p += StringLen(pat);
   int e = StringFind(j, "\"", p);
   return(e < 0 ? "" : StringSubstr(j, p, e - p));
  }

string JRaw(const string j, const string key)
  {
   string pat = "\"" + key + "\":";
   int p = StringFind(j, pat);
   if(p < 0)
      return("");
   p += StringLen(pat);
   int n = StringLen(j), e = p;
   while(e < n)
     {
      ushort c = StringGetCharacter(j, e);
      if(c == ',' || c == '}' || c == ']')
         break;
      e++;
     }
   return(StringSubstr(j, p, e - p));
  }

double JNum(const string j, const string key) { return(StringToDouble(JRaw(j, key))); }
long   JInt(const string j, const string key) { return(StringToInteger(JRaw(j, key))); }

int JStrArr(const string j, const string key, string &out[])
  {
   ArrayResize(out, 0);
   string pat = "\"" + key + "\":[";
   int p = StringFind(j, pat);
   if(p < 0)
      return(0);
   p += StringLen(pat);
   int e = StringFind(j, "]", p);
   if(e <= p)
      return(0);
   string parts[];
   int n = StringSplit(StringSubstr(j, p, e - p), ',', parts);
   for(int i = 0; i < n; i++)
     {
      string s = parts[i];
      StringReplace(s, "\"", "");
      StringTrimLeft(s);
      StringTrimRight(s);
      if(s != "")
        {
         int k = ArraySize(out);
         ArrayResize(out, k + 1);
         out[k] = s;
        }
     }
   return(ArraySize(out));
  }

string Esc(string s)
  {
   StringReplace(s, "\\", "\\\\");
   StringReplace(s, "\"", "\\\"");
   return(s);
  }

string Num(const double v, const int digits) { return(DoubleToString(v, digits)); }

//+------------------------------------------------------------------+
//| Socket                                                           |
//+------------------------------------------------------------------+
void Disconnect()
  {
   if(g_sock != INVALID_HANDLE)
      SocketClose(g_sock);
   g_sock = INVALID_HANDLE;
   ArrayResize(g_rx, 0);
   ArrayResize(g_syms, 0);
   ArrayResize(g_lastMsc, 0);
   ArrayResize(g_hSym, 0);
   ArrayResize(g_hTf, 0);
   ArrayResize(g_hCount, 0);
   ArrayResize(g_hBefore, 0);
   ArrayResize(g_hTries, 0);
   ArrayResize(g_qSym, 0);
   ArrayResize(g_qTf, 0);
   ArrayResize(g_qCount, 0);
   ArrayResize(g_qSkip, 0);
   ArrayResize(g_qRow, 0);
   g_dNext = -1;
   g_nextConnect = GetTickCount64() + RECONNECT_MS;
  }

//--- várias linhas de uma vez: menos chamadas ao socket
bool Send(const string text)
  {
   if(g_sock == INVALID_HANDLE || text == "")
      return(false);
   uchar buf[];
   int n = StringToCharArray(text, buf, 0, WHOLE_ARRAY, CP_UTF8) - 1; // sem o 0 final
   int sent = 0;
   while(sent < n)
     {
      int r;
      if(sent == 0)
         r = SocketSend(g_sock, buf, n);
      else
        {
         uchar rest[];
         ArrayCopy(rest, buf, 0, sent, n - sent);
         r = SocketSend(g_sock, rest, n - sent);
        }
      if(r <= 0)
        {
         PrintFormat("TerminalBridge: envio falhou (erro %d), reconectando", GetLastError());
         Disconnect();
         return(false);
        }
      sent += r;
     }
   return(true);
  }

bool Connect()
  {
   g_sock = SocketCreate();
   if(g_sock == INVALID_HANDLE)
     {
      g_nextConnect = GetTickCount64() + RECONNECT_MS;
      return(false);
     }
   ResetLastError();
   if(!SocketConnect(g_sock, InpHost, InpPort, 300))
     {
      int err = GetLastError();
      SocketClose(g_sock);
      g_sock = INVALID_HANDLE;
      g_nextConnect = GetTickCount64() + RECONNECT_MS;
      if(err == 4014 && !g_hintShown)
        {
         g_hintShown = true;
         Print("TerminalBridge: adicione ", InpHost,
               " em Ferramentas > Opções > Expert Advisors > 'Permitir WebRequest para as URLs listadas'.");
        }
      return(false);
     }
   ArrayResize(g_rx, 0);
   g_lastPositions = "";
   g_lastOrders = "";
   g_lastAccount = "";
   Print("TerminalBridge: conectado em ", InpHost, ":", InpPort);
   SendHello();
   SendState(true);
   return(true);
  }

//+------------------------------------------------------------------+
//| Mensagens para o app                                             |
//+------------------------------------------------------------------+
void SendError(const string msg)
  {
   Send("{\"t\":\"error\",\"msg\":\"" + Esc(msg) + "\"}\n");
  }

void SendHello()
  {
   long mode = AccountInfoInteger(ACCOUNT_TRADE_MODE);
   string kind = mode == ACCOUNT_TRADE_MODE_DEMO ? "demo" : (mode == ACCOUNT_TRADE_MODE_CONTEST ? "contest" : "real");
   bool netting = AccountInfoInteger(ACCOUNT_MARGIN_MODE) != ACCOUNT_MARGIN_MODE_RETAIL_HEDGING;
   Send("{\"t\":\"hello\",\"symbol\":\"" + Esc(_Symbol) + "\",\"digits\":" + IntegerToString(_Digits) +
        ",\"server\":\"" + Esc(AccountInfoString(ACCOUNT_SERVER)) + "\",\"login\":" +
        IntegerToString(AccountInfoInteger(ACCOUNT_LOGIN)) + ",\"account\":\"" + kind + "\",\"version\":" +
        IntegerToString(BRIDGE_VERSION) + ",\"netting\":" + (netting ? "true" : "false") + "}\n");
  }

void SendSymbol(const string sym)
  {
   int d = (int)SymbolInfoInteger(sym, SYMBOL_DIGITS);
   Send("{\"t\":\"symbol\",\"symbol\":\"" + Esc(sym) + "\",\"digits\":" + IntegerToString(d) +
        ",\"tick_size\":" + Num(SymbolInfoDouble(sym, SYMBOL_TRADE_TICK_SIZE), d) +
        ",\"tick_value_profit\":" + Num(SymbolInfoDouble(sym, SYMBOL_TRADE_TICK_VALUE_PROFIT), 10) +
        ",\"tick_value_loss\":" + Num(SymbolInfoDouble(sym, SYMBOL_TRADE_TICK_VALUE_LOSS), 10) +
        ",\"vol_min\":" + Num(SymbolInfoDouble(sym, SYMBOL_VOLUME_MIN), 8) +
        ",\"vol_max\":" + Num(SymbolInfoDouble(sym, SYMBOL_VOLUME_MAX), 8) +
        ",\"vol_step\":" + Num(SymbolInfoDouble(sym, SYMBOL_VOLUME_STEP), 8) + "}\n");
  }

void SendResult(const long id, const bool ok, const uint retcode, const string msg, const ulong ticket, const double price)
  {
   Send("{\"t\":\"trade_result\",\"id\":" + IntegerToString(id) + ",\"ok\":" + (ok ? "true" : "false") +
        ",\"retcode\":" + IntegerToString(retcode) + ",\"msg\":\"" + Esc(msg) + "\",\"ticket\":" +
        IntegerToString((long)ticket) + ",\"price\":" + DoubleToString(price, 8) + "}\n");
  }

//--- realizado da conta inteira; relê só na virada do dia ou quando muda um negócio
void UpdateDailyResult()
  {
   datetime now = TimeTradeServer();
   datetime last = TimeCurrent();
   if(last > now)
      now = last;
   if(now <= 0)
      return;
   datetime day = now - now % 86400;
   if(day != g_dayStart)
     {
      g_dayStart = day;
      g_dayRealized = 0.0;
      g_dayDirty = true;
      g_dayReady = false;
      g_nextDayRetry = 0;
     }
   if(!g_dayDirty || GetTickCount64() < g_nextDayRetry)
      return;
   g_nextDayRetry = GetTickCount64() + 1000;
   if(!HistorySelect(g_dayStart, now))
      return;
   double realized = 0.0;
   for(int i = 0; i < HistoryDealsTotal(); i++)
     {
      ulong ticket = HistoryDealGetTicket(i);
      if(ticket == 0)
         continue;
      long type = HistoryDealGetInteger(ticket, DEAL_TYPE);
      if(type != DEAL_TYPE_BUY && type != DEAL_TYPE_SELL &&
         type != DEAL_TYPE_COMMISSION && type != DEAL_TYPE_COMMISSION_DAILY &&
         type != DEAL_TYPE_COMMISSION_MONTHLY && type != DEAL_TYPE_COMMISSION_AGENT_DAILY &&
         type != DEAL_TYPE_COMMISSION_AGENT_MONTHLY)
         continue;
      realized += HistoryDealGetDouble(ticket, DEAL_PROFIT) + HistoryDealGetDouble(ticket, DEAL_COMMISSION) +
                  HistoryDealGetDouble(ticket, DEAL_SWAP) + HistoryDealGetDouble(ticket, DEAL_FEE);
     }
   g_dayRealized = realized;
   g_dayDirty = false;
   g_dayReady = true;
  }

//--- posições, ordens, conta e resultado do dia: só o que mudou
void SendState(const bool force)
  {
   string out = "";

   string pos = "{\"t\":\"positions\",\"positions\":[";
   int total = PositionsTotal();
   for(int i = 0; i < total; i++)
     {
      ulong ticket = PositionGetTicket(i);
      if(ticket == 0)
         continue;
      string sym = PositionGetString(POSITION_SYMBOL);
      int d = (int)SymbolInfoInteger(sym, SYMBOL_DIGITS);
      bool buy = PositionGetInteger(POSITION_TYPE) == POSITION_TYPE_BUY;
      if(StringGetCharacter(pos, StringLen(pos) - 1) != '[')
         StringAdd(pos, ",");
      StringAdd(pos, "{\"ticket\":" + IntegerToString((long)ticket) + ",\"symbol\":\"" + Esc(sym) +
                "\",\"side\":\"" + (buy ? "buy" : "sell") + "\",\"volume\":" + Num(PositionGetDouble(POSITION_VOLUME), 8) +
                ",\"price\":" + Num(PositionGetDouble(POSITION_PRICE_OPEN), d) +
                ",\"sl\":" + Num(PositionGetDouble(POSITION_SL), d) + ",\"tp\":" + Num(PositionGetDouble(POSITION_TP), d) +
                ",\"profit\":" + Num(PositionGetDouble(POSITION_PROFIT) + PositionGetDouble(POSITION_SWAP), 2) + "}");
     }
   StringAdd(pos, "]}\n");
   if(force || pos != g_lastPositions)
     {
      StringAdd(out, pos);
      g_lastPositions = pos;
     }

   string ord = "{\"t\":\"orders\",\"orders\":[";
   total = OrdersTotal();
   for(int i = 0; i < total; i++)
     {
      ulong ticket = OrderGetTicket(i);
      if(ticket == 0)
         continue;
      ENUM_ORDER_TYPE type = (ENUM_ORDER_TYPE)OrderGetInteger(ORDER_TYPE);
      string side, kind;
      switch(type)
        {
         case ORDER_TYPE_BUY_LIMIT:  side = "buy";  kind = "limit"; break;
         case ORDER_TYPE_SELL_LIMIT: side = "sell"; kind = "limit"; break;
         case ORDER_TYPE_BUY_STOP:   side = "buy";  kind = "stop";  break;
         case ORDER_TYPE_SELL_STOP:  side = "sell"; kind = "stop";  break;
         default: continue; // stop-limit e ordens a mercado em trânsito
        }
      string sym = OrderGetString(ORDER_SYMBOL);
      int d = (int)SymbolInfoInteger(sym, SYMBOL_DIGITS);
      if(StringGetCharacter(ord, StringLen(ord) - 1) != '[')
         StringAdd(ord, ",");
      StringAdd(ord, "{\"ticket\":" + IntegerToString((long)ticket) + ",\"symbol\":\"" + Esc(sym) +
                "\",\"side\":\"" + side + "\",\"kind\":\"" + kind + "\",\"volume\":" +
                Num(OrderGetDouble(ORDER_VOLUME_CURRENT), 8) + ",\"price\":" + Num(OrderGetDouble(ORDER_PRICE_OPEN), d) +
                ",\"sl\":" + Num(OrderGetDouble(ORDER_SL), d) + ",\"tp\":" + Num(OrderGetDouble(ORDER_TP), d) + "}");
     }
   StringAdd(ord, "]}\n");
   if(force || ord != g_lastOrders)
     {
      StringAdd(out, ord);
      g_lastOrders = ord;
     }

   bool allowed = TerminalInfoInteger(TERMINAL_TRADE_ALLOWED) && MQLInfoInteger(MQL_TRADE_ALLOWED) &&
                  AccountInfoInteger(ACCOUNT_TRADE_ALLOWED) && AccountInfoInteger(ACCOUNT_TRADE_EXPERT);
   string acc = "{\"t\":\"account\",\"balance\":" + Num(AccountInfoDouble(ACCOUNT_BALANCE), 2) +
                ",\"equity\":" + Num(AccountInfoDouble(ACCOUNT_EQUITY), 2) +
                ",\"margin_free\":" + Num(AccountInfoDouble(ACCOUNT_MARGIN_FREE), 2) +
                ",\"currency\":\"" + Esc(AccountInfoString(ACCOUNT_CURRENCY)) + "\",\"trade_allowed\":" +
                (allowed ? "true" : "false") + "}\n";
   if(force || acc != g_lastAccount)
     {
      StringAdd(out, acc);
      g_lastAccount = acc;
     }
   UpdateDailyResult();
   string daily = "{\"t\":\"daily_result\",\"day_start\":" + IntegerToString((long)g_dayStart) +
                  ",\"realized\":" + (g_dayReady && !g_dayDirty ? Num(g_dayRealized, 2) : "null") +
                  ",\"floating\":" + Num(AccountInfoDouble(ACCOUNT_PROFIT), 2) +
                  ",\"currency\":\"" + Esc(AccountInfoString(ACCOUNT_CURRENCY)) + "\"}\n";
   if(force || daily != g_lastDaily)
     {
      StringAdd(out, daily);
      g_lastDaily = daily;
     }
   Send(out);
  }

//--- ticks novos de cada símbolo assinado desde o último enviado
void PumpTicks()
  {
   string out = "";
   int n = ArraySize(g_syms);
   for(int i = 0; i < n && g_sock != INVALID_HANDLE; i++)
     {
      if(g_lastMsc[i] == 0)
        {
         // nenhum tick ainda: começa do atual, nunca do histórico inteiro
         MqlTick t;
         if(SymbolInfoTick(g_syms[i], t))
            g_lastMsc[i] = t.time_msc;
         continue;
        }
      MqlTick ticks[];
      int got = CopyTicks(g_syms[i], ticks, COPY_TICKS_ALL, (ulong)g_lastMsc[i] + 1, MAX_TICKS);
      if(got <= 0)
         continue;
      int d = (int)SymbolInfoInteger(g_syms[i], SYMBOL_DIGITS);
      string head = "{\"t\":\"tick\",\"symbol\":\"" + Esc(g_syms[i]) + "\",\"time_msc\":";
      for(int k = 0; k < got; k++)
        {
         if(ticks[k].time_msc <= g_lastMsc[i] || ticks[k].bid <= 0.0)
            continue;
         // CFD sem volume real: cada tick conta 1, como o tick_volume das barras
         double vol = ticks[k].volume_real > 0.0 ? ticks[k].volume_real : 1.0;
         StringAdd(out, head + IntegerToString(ticks[k].time_msc) + ",\"bid\":" + Num(ticks[k].bid, d) +
                   ",\"ask\":" + Num(ticks[k].ask, d) + ",\"volume\":" + Num(vol, 2) + "}\n");
        }
      g_lastMsc[i] = ticks[got - 1].time_msc;
     }
   Send(out);
  }

ENUM_TIMEFRAMES TfFromLabel(const string tf)
  {
   if(tf == "M1")  return(PERIOD_M1);
   if(tf == "M5")  return(PERIOD_M5);
   if(tf == "M15") return(PERIOD_M15);
   if(tf == "M30") return(PERIOD_M30);
   if(tf == "H1")  return(PERIOD_H1);
   if(tf == "H4")  return(PERIOD_H4);
   if(tf == "D1")  return(PERIOD_D1);
   if(tf == "W1")  return(PERIOD_W1);
   return(PERIOD_CURRENT);
  }

//--- false: histórico ainda carregando
string BarsHead(const string sym, const string tf, const int digits, const long before)
  {
   return("{\"t\":\"bars\",\"symbol\":\"" + Esc(sym) + "\",\"tf\":\"" + tf + "\",\"digits\":" + IntegerToString(digits) +
          (before > 0 ? ",\"before\":" + IntegerToString(before) : "") + ",\"bars\":[");
  }

//--- false: histórico ainda carregando. before > 0: os count candles anteriores a before; lista vazia
//--- quando não há mais nada antes (o app para de pedir)
bool SendHistory(const string sym, const string tf, const int count, const long before)
  {
   ENUM_TIMEFRAMES period = TfFromLabel(tf);
   if(period == PERIOD_CURRENT)
     {
      SendError("timeframe desconhecido: " + tf);
      return(true);
     }
   if(!SymbolSelect(sym, true))
     {
      SendError("símbolo desconhecido: " + sym);
      return(true);
     }
   MqlRates rates[];
   int d = (int)SymbolInfoInteger(sym, SYMBOL_DIGITS);
   int n = before > 0 ? CopyRates(sym, period, (datetime)(before - 1), count, rates) : CopyRates(sym, period, 0, count, rates);
   if(n <= 0)
     {
      long first = SeriesInfoInteger(sym, period, SERIES_SERVER_FIRSTDATE);
      if(before > 0 && first > 0 && first >= before)
        {
         Send(BarsHead(sym, tf, d, before) + "]}\n");
         return(true);
        }
      return(false);
     }
   // espaço reservado e partes acrescentadas uma a uma: sem realocar nem criar strings temporárias
   // a cada candle (5000 candles levavam ~1,2 s concatenando)
   string out = BarsHead(sym, tf, d, before);
   StringReserve(out, StringLen(out) + n * (6 * (d + 12)) + 8);
   for(int i = 0; i < n; i++)
     {
      StringAdd(out, i > 0 ? ",[" : "[");
      StringAdd(out, IntegerToString((long)rates[i].time));
      StringAdd(out, ",");
      StringAdd(out, DoubleToString(rates[i].open, d));
      StringAdd(out, ",");
      StringAdd(out, DoubleToString(rates[i].high, d));
      StringAdd(out, ",");
      StringAdd(out, DoubleToString(rates[i].low, d));
      StringAdd(out, ",");
      StringAdd(out, DoubleToString(rates[i].close, d));
      StringAdd(out, ",");
      StringAdd(out, IntegerToString(rates[i].tick_volume));
      StringAdd(out, "]");
     }
   StringAdd(out, "]}\n");
   if(before == 0)
      SendSymbol(sym);
   Send(out);
   return(true);
  }

void Subscribe(string &syms[])
  {
   int n = ArraySize(syms);
   ArrayResize(g_syms, n);
   ArrayResize(g_lastMsc, n);
   for(int i = 0; i < n; i++)
     {
      g_syms[i] = syms[i];
      SymbolSelect(syms[i], true);
      MqlTick t;
      g_lastMsc[i] = SymbolInfoTick(syms[i], t) ? t.time_msc : 0;
     }
  }

//+------------------------------------------------------------------+
//| Ordens (assíncronas: o resultado chega em OnTradeTransaction)    |
//+------------------------------------------------------------------+
ENUM_ORDER_TYPE_FILLING Filling(const string sym)
  {
   long f = SymbolInfoInteger(sym, SYMBOL_FILLING_MODE);
   if((f & SYMBOL_FILLING_FOK) != 0)
      return(ORDER_FILLING_FOK);
   if((f & SYMBOL_FILLING_IOC) != 0)
      return(ORDER_FILLING_IOC);
   return(ORDER_FILLING_RETURN);
  }

double NormPrice(const string sym, const double price)
  {
   if(price <= 0.0)
      return(0.0);
   double tick = SymbolInfoDouble(sym, SYMBOL_TRADE_TICK_SIZE);
   int d = (int)SymbolInfoInteger(sym, SYMBOL_DIGITS);
   if(tick > 0.0)
      return(NormalizeDouble(MathRound(price / tick) * tick, d));
   return(NormalizeDouble(price, d));
  }

string RetcodeText(const uint rc)
  {
   switch(rc)
     {
      case TRADE_RETCODE_DONE:           return("executada");
      case TRADE_RETCODE_DONE_PARTIAL:   return("executada parcialmente");
      case TRADE_RETCODE_PLACED:         return("ordem colocada");
      case TRADE_RETCODE_REQUOTE:        return("requote");
      case TRADE_RETCODE_REJECT:         return("rejeitada");
      case TRADE_RETCODE_CANCEL:         return("cancelada pelo operador");
      case TRADE_RETCODE_TIMEOUT:        return("tempo esgotado");
      case TRADE_RETCODE_INVALID:        return("requisição inválida");
      case TRADE_RETCODE_INVALID_VOLUME: return("volume inválido");
      case TRADE_RETCODE_INVALID_PRICE:  return("preço inválido");
      case TRADE_RETCODE_INVALID_STOPS:  return("stops inválidos");
      case TRADE_RETCODE_TRADE_DISABLED: return("negociação desabilitada");
      case TRADE_RETCODE_MARKET_CLOSED:  return("mercado fechado");
      case TRADE_RETCODE_NO_MONEY:       return("sem margem");
      case TRADE_RETCODE_PRICE_CHANGED:  return("preço mudou");
      case TRADE_RETCODE_PRICE_OFF:      return("sem cotação");
      case TRADE_RETCODE_INVALID_FILL:   return("tipo de preenchimento inválido");
      case TRADE_RETCODE_CONNECTION:     return("sem conexão com o servidor");
      case TRADE_RETCODE_CLIENT_DISABLES_AT: return("Algo Trading desligado no terminal");
      case TRADE_RETCODE_SERVER_DISABLES_AT: return("Algo Trading desligado pelo servidor");
      case TRADE_RETCODE_LIMIT_ORDERS:   return("limite de ordens");
      case TRADE_RETCODE_LIMIT_VOLUME:   return("limite de volume");
      case TRADE_RETCODE_INVALID_ORDER:  return("ordem inválida ou proibida");
      case TRADE_RETCODE_POSITION_CLOSED: return("posição já fechada");
      case TRADE_RETCODE_FIFO_CLOSE:     return("fechamento só por FIFO");
      case TRADE_RETCODE_HEDGE_PROHIBITED: return("hedge proibido");
     }
   return("retcode " + IntegerToString(rc));
  }

bool OkRetcode(const uint rc)
  {
   return(rc == TRADE_RETCODE_DONE || rc == TRADE_RETCODE_DONE_PARTIAL || rc == TRADE_RETCODE_PLACED);
  }

void Submit(const long id, MqlTradeRequest &rq)
  {
   rq.magic = InpMagic;
   if(rq.comment == "")
      rq.comment = "mt5-terminal";
   MqlTradeResult rs;
   ZeroMemory(rs);
   ResetLastError();
   if(!OrderSendAsync(rq, rs))
     {
      uint rc = rs.retcode;
      string msg = rc != 0 ? RetcodeText(rc) : "OrderSendAsync falhou (erro " + IntegerToString(GetLastError()) + ")";
      SendResult(id, false, rc, msg, 0, 0.0);
      return;
     }
   int k = ArraySize(g_reqIds);
   ArrayResize(g_reqIds, k + 1);
   ArrayResize(g_cliIds, k + 1);
   g_reqIds[k] = rs.request_id;
   g_cliIds[k] = id;
  }

void DoOrder(const string j)
  {
   long id = JInt(j, "id");
   string sym = JStr(j, "symbol");
   bool buy = JStr(j, "side") == "buy";
   string kind = JStr(j, "kind");
   if(!SymbolSelect(sym, true))
     {
      SendResult(id, false, 0, "símbolo desconhecido: " + sym, 0, 0.0);
      return;
     }
   MqlTick tick;
   if(!SymbolInfoTick(sym, tick) || tick.bid <= 0.0)
     {
      SendResult(id, false, TRADE_RETCODE_PRICE_OFF, RetcodeText(TRADE_RETCODE_PRICE_OFF), 0, 0.0);
      return;
     }
   MqlTradeRequest rq;
   ZeroMemory(rq);
   rq.symbol = sym;
   rq.volume = JNum(j, "volume");
   rq.sl = NormPrice(sym, JNum(j, "sl"));
   rq.tp = NormPrice(sym, JNum(j, "tp"));
   rq.deviation = InpDeviation;
   rq.type_filling = Filling(sym);
   if(kind == "market")
     {
      rq.action = TRADE_ACTION_DEAL;
      rq.type = buy ? ORDER_TYPE_BUY : ORDER_TYPE_SELL;
      rq.price = buy ? tick.ask : tick.bid;
     }
   else
     {
      bool limit = kind == "limit";
      rq.action = TRADE_ACTION_PENDING;
      rq.type = buy ? (limit ? ORDER_TYPE_BUY_LIMIT : ORDER_TYPE_BUY_STOP) : (limit ? ORDER_TYPE_SELL_LIMIT : ORDER_TYPE_SELL_STOP);
      rq.price = NormPrice(sym, JNum(j, "price"));
      rq.type_time = ORDER_TIME_GTC;
     }
   Submit(id, rq);
  }

bool ClosePosition(const long id, const ulong ticket)
  {
   if(!PositionSelectByTicket(ticket))
     {
      SendResult(id, false, TRADE_RETCODE_POSITION_CLOSED, "posição não encontrada", ticket, 0.0);
      return(false);
     }
   string sym = PositionGetString(POSITION_SYMBOL);
   bool buy = PositionGetInteger(POSITION_TYPE) == POSITION_TYPE_BUY;
   MqlTick tick;
   SymbolInfoTick(sym, tick);
   MqlTradeRequest rq;
   ZeroMemory(rq);
   rq.action = TRADE_ACTION_DEAL;
   rq.position = ticket;
   rq.symbol = sym;
   rq.volume = PositionGetDouble(POSITION_VOLUME);
   rq.type = buy ? ORDER_TYPE_SELL : ORDER_TYPE_BUY;
   rq.price = buy ? tick.bid : tick.ask;
   rq.deviation = InpDeviation;
   rq.type_filling = Filling(sym);
   Submit(id, rq);
   return(true);
  }

bool CancelOrder(const long id, const ulong ticket)
  {
   if(!OrderSelect(ticket))
     {
      SendResult(id, false, TRADE_RETCODE_INVALID_ORDER, "ordem não encontrada", ticket, 0.0);
      return(false);
     }
   MqlTradeRequest rq;
   ZeroMemory(rq);
   rq.action = TRADE_ACTION_REMOVE;
   rq.order = ticket;
   Submit(id, rq);
   return(true);
  }

//--- ordem pendente: preço, stop e alvo; posição: stop e alvo. Valores absolutos, 0 = sem
void Modify(const string j)
  {
   long id = JInt(j, "id");
   ulong ticket = (ulong)JInt(j, "ticket");
   MqlTradeRequest rq;
   ZeroMemory(rq);
   if(PositionSelectByTicket(ticket))
     {
      string sym = PositionGetString(POSITION_SYMBOL);
      rq.action = TRADE_ACTION_SLTP;
      rq.position = ticket;
      rq.symbol = sym;
      rq.sl = NormPrice(sym, JNum(j, "sl"));
      rq.tp = NormPrice(sym, JNum(j, "tp"));
     }
   else if(OrderSelect(ticket))
     {
      string sym = OrderGetString(ORDER_SYMBOL);
      rq.action = TRADE_ACTION_MODIFY;
      rq.order = ticket;
      rq.symbol = sym;
      rq.price = NormPrice(sym, JNum(j, "price"));
      rq.sl = NormPrice(sym, JNum(j, "sl"));
      rq.tp = NormPrice(sym, JNum(j, "tp"));
      rq.type_time = (ENUM_ORDER_TYPE_TIME)OrderGetInteger(ORDER_TYPE_TIME);
      rq.expiration = (datetime)OrderGetInteger(ORDER_TIME_EXPIRATION);
     }
   else
     {
      SendResult(id, false, TRADE_RETCODE_INVALID_ORDER, "ordem ou posição não encontrada", ticket, 0.0);
      return;
     }
   Submit(id, rq);
  }

void Flatten(const long id, const string sym)
  {
   ulong tickets[];
   for(int i = PositionsTotal() - 1; i >= 0; i--)
     {
      ulong t = PositionGetTicket(i);
      if(t != 0 && PositionGetString(POSITION_SYMBOL) == sym)
        {
         int k = ArraySize(tickets);
         ArrayResize(tickets, k + 1);
         tickets[k] = t;
        }
     }
   ulong orders[];
   for(int i = OrdersTotal() - 1; i >= 0; i--)
     {
      ulong t = OrderGetTicket(i);
      if(t != 0 && OrderGetString(ORDER_SYMBOL) == sym)
        {
         int k = ArraySize(orders);
         ArrayResize(orders, k + 1);
         orders[k] = t;
        }
     }
   if(ArraySize(tickets) + ArraySize(orders) == 0)
     {
      SendResult(id, true, 0, "nada a zerar", 0, 0.0);
      return;
     }
   // ordens primeiro, para nenhuma abrir posição enquanto zera
   for(int i = 0; i < ArraySize(orders); i++)
      CancelOrder(id, orders[i]);
   for(int i = 0; i < ArraySize(tickets); i++)
      ClosePosition(id, tickets[i]);
  }

//+------------------------------------------------------------------+
//| Diagnóstico: valores dos indicadores que estão nos gráficos do   |
//| MT5, para conferir os cálculos do app                            |
//+------------------------------------------------------------------+
long FindChart(const string sym, const ENUM_TIMEFRAMES period)
  {
   for(long id = ChartFirst(); id >= 0; id = ChartNext(id))
      if(ChartSymbol(id) == sym && ChartPeriod(id) == period)
         return(id);
   return(-1);
  }

void Probe(const string j)
  {
   long id = JInt(j, "id");
   string sym = JStr(j, "symbol"), tf = JStr(j, "tf"), name = JStr(j, "indicator");
   int buffer = (int)JInt(j, "buffer"), count = (int)JInt(j, "count");
   ENUM_TIMEFRAMES period = TfFromLabel(tf);
   long chart = FindChart(sym, period);
   int handle = INVALID_HANDLE;
   if(chart >= 0)
      for(int w = 0; w < (int)ChartGetInteger(chart, CHART_WINDOWS_TOTAL) && handle == INVALID_HANDLE; w++)
         for(int k = 0; k < ChartIndicatorsTotal(chart, w); k++)
           {
            string shortName = ChartIndicatorName(chart, w, k);
            if(StringFind(shortName, name) == 0)
              {
               handle = ChartIndicatorGet(chart, w, shortName);
               break;
              }
           }
   string head = "{\"t\":\"probe\",\"id\":" + IntegerToString(id) + ",\"indicator\":\"" + Esc(name) +
                 "\",\"buffer\":" + IntegerToString(buffer);
   double values[];
   datetime times[];
   int n = handle == INVALID_HANDLE ? -1 : CopyBuffer(handle, buffer, 0, count, values);
   int nt = n > 0 ? CopyTime(sym, period, 0, n, times) : -1;
   if(handle != INVALID_HANDLE)
      IndicatorRelease(handle);
   if(n <= 0 || nt != n)
     {
      Send(head + ",\"times\":[],\"values\":[]}\n");
      return;
     }
   string ts = "", vs = "";
   for(int i = 0; i < n; i++)
     {
      if(i > 0)
        {
         StringAdd(ts, ",");
         StringAdd(vs, ",");
        }
      StringAdd(ts, IntegerToString((long)times[i]));
      StringAdd(vs, values[i] == EMPTY_VALUE || !MathIsValidNumber(values[i]) ? "null" : DoubleToString(values[i], 10));
     }
   Send(head + ",\"times\":[" + ts + "],\"values\":[" + vs + "]}\n");
  }

void ProbeObjects(const string j)
  {
   long id = JInt(j, "id");
   string sym = JStr(j, "symbol"), prefix = JStr(j, "prefix");
   long chart = FindChart(sym, TfFromLabel(JStr(j, "tf")));
   string items = "";
   if(chart >= 0)
     {
      int total = ObjectsTotal(chart, -1, -1);
      for(int i = 0; i < total; i++)
        {
         string name = ObjectName(chart, i, -1, -1);
         if(StringFind(name, prefix) != 0)
            continue;
         if(items != "")
            StringAdd(items, ",");
         StringAdd(items, "{\"name\":\"" + Esc(name) + "\",\"text\":\"" + Esc(ObjectGetString(chart, name, OBJPROP_TEXT)) +
                   "\",\"price\":" + DoubleToString(ObjectGetDouble(chart, name, OBJPROP_PRICE), 10) + "}");
        }
     }
   Send("{\"t\":\"objects\",\"id\":" + IntegerToString(id) + ",\"items\":[" + items + "]}\n");
  }

//+------------------------------------------------------------------+
//| Delta de volume (compras - vendas) por candle, pela regra do tick |
//| sobre o preço médio (bid+ask)/2: subiu = compra, caiu = venda,     |
//| igual = mesmo lado do anterior; cada tick conta 1 (CFD não tem     |
//| lado nem volume real). Com flags de compra/venda e volume real     |
//| (bolsa), usa os dados reais. A regra recomeça a cada candle.       |
//+------------------------------------------------------------------+
struct SDelta
  {
   double buy, sell, prevMid;
   int    dir;
   long   ticks;
   // POC: volume por nível de altura row (chave = floor(bid / row)); a POC é o primeiro nível a
   // passar estritamente o volume da POC anterior
   double row;
   long   keys[];
   double vols[];
   int    poc;
   void   Reset(const double r) { buy = 0; sell = 0; prevMid = 0; dir = 0; ticks = 0; row = r; ArrayResize(keys, 0); ArrayResize(vols, 0); poc = -1; }
   void   Count(const double bid, const double v)
     {
      if(row <= 0.0 || v <= 0.0)
         return;
      long key = (long)MathFloor(bid / row + 1e-9);
      int n = ArraySize(keys), r = -1;
      for(int i = 0; i < n && r < 0; i++)
         if(keys[i] == key)
            r = i;
      if(r < 0)
        {
         ArrayResize(keys, n + 1, 16);
         ArrayResize(vols, n + 1, 16);
         keys[n] = key;
         vols[n] = 0;
         r = n;
        }
      vols[r] += v;
      if(poc < 0 || vols[r] > vols[poc])
         poc = r;
     }
   double PocPrice() { return(poc < 0 ? 0.0 : ((double)keys[poc] + 0.5) * row); }
   void   Add(const MqlTick &tk)
     {
      if(tk.bid <= 0.0)
         return;
      ticks++;
      bool isBuy = (tk.flags & TICK_FLAG_BUY) != 0, isSell = (tk.flags & TICK_FLAG_SELL) != 0;
      if((isBuy || isSell) && tk.volume_real > 0)
        {
         if(isBuy)  buy += tk.volume_real;
         if(isSell) sell += tk.volume_real;
         Count(tk.bid, tk.volume_real);
         return;
        }
      double mid = tk.ask > 0.0 ? (tk.bid + tk.ask) / 2.0 : tk.bid;
      if(prevMid > 0.0)
        {
         if(mid > prevMid)      dir = 1;
         else if(mid < prevMid) dir = -1;
        }
      prevMid = mid;
      if(dir > 0)      buy += 1.0;
      else if(dir < 0) sell += 1.0;
      // ticks antes do primeiro movimento não têm lado: ficam fora do perfil, como do delta
      if(dir != 0)
         Count(tk.bid, 1.0);
     }
  };

//--- começa o próximo pedido da fila: os count candles fechados mais recentes
void StartDelta()
  {
   while(g_dNext < 0 && ArraySize(g_qSym) > 0)
     {
      g_dSym = g_qSym[0];
      g_dTf = g_qTf[0];
      int count = g_qCount[0], skip = g_qSkip[0];
      g_dRow = g_qRow[0];
      ArrayRemove(g_qSym, 0, 1);
      ArrayRemove(g_qTf, 0, 1);
      ArrayRemove(g_qCount, 0, 1);
      ArrayRemove(g_qSkip, 0, 1);
      ArrayRemove(g_qRow, 0, 1);
      ENUM_TIMEFRAMES period = TfFromLabel(g_dTf);
      if(period == PERIOD_CURRENT || !SymbolSelect(g_dSym, true))
         continue;
      g_dPeriodMsc = (long)PeriodSeconds(period) * 1000;
      int n = CopyTime(g_dSym, period, 1 + MathMax(skip, 0), count, g_dTimes);
      g_dNext = n > 0 ? n - 1 : -1;
      g_dTries = 0;
     }
  }

//--- um bloco: os candles pendentes seguidos que cabem em DELTA_CHUNK_MSC, numa só leitura de ticks
void DeltaStep()
  {
   StartDelta();
   if(g_dNext < 0)
      return;
   int k = g_dNext, j = k;
   long endMsc = (long)g_dTimes[k] * 1000 + g_dPeriodMsc;
   while(j > 0 && endMsc - (long)g_dTimes[j - 1] * 1000 <= DELTA_CHUNK_MSC)
      j--;
   MqlTick ticks[];
   int got = CopyTicksRange(g_dSym, ticks, COPY_TICKS_ALL, (ulong)g_dTimes[j] * 1000, (ulong)endMsc - 1);
   if(got <= 0 && ++g_dTries < DELTA_TRIES)
      return; // histórico de ticks ainda baixando: tenta no próximo ciclo
   g_dTries = 0;
   string out = "";
   int pos = 0;
   for(int m = j; m <= k && got > 0; m++)
     {
      long from = (long)g_dTimes[m] * 1000, to = from + g_dPeriodMsc;
      SDelta d;
      d.Reset(g_dRow);
      while(pos < got && (long)ticks[pos].time_msc < from)
         pos++;
      while(pos < got && (long)ticks[pos].time_msc < to)
         d.Add(ticks[pos++]);
      if(d.ticks == 0)
         continue;
      if(out != "")
         StringAdd(out, ",");
      int digits = (int)SymbolInfoInteger(g_dSym, SYMBOL_DIGITS);
      StringAdd(out, "[" + IntegerToString((long)g_dTimes[m]) + "," + DoubleToString(d.buy, 2) + "," + DoubleToString(d.sell, 2) +
                (g_dRow > 0 ? "," + DoubleToString(d.PocPrice(), digits + 2) : "") + "]");
     }
   if(out != "")
      Send("{\"t\":\"delta\",\"symbol\":\"" + Esc(g_dSym) + "\",\"tf\":\"" + g_dTf + "\",\"bars\":[" + out + "]}\n");
   g_dNext = j - 1;
  }

void Handle(const string line)
  {
   string t = JStr(line, "t");
   if(t == "order")
      DoOrder(line);
   else if(t == "close")
      ClosePosition(JInt(line, "id"), (ulong)JInt(line, "ticket"));
   else if(t == "cancel")
      CancelOrder(JInt(line, "id"), (ulong)JInt(line, "ticket"));
   else if(t == "modify")
      Modify(line);
   else if(t == "flatten")
      Flatten(JInt(line, "id"), JStr(line, "symbol"));
   else if(t == "subscribe")
     {
      string syms[];
      JStrArr(line, "symbols", syms);
      Subscribe(syms);
     }
   else if(t == "history")
     {
      int k = ArraySize(g_hSym);
      ArrayResize(g_hSym, k + 1);
      ArrayResize(g_hTf, k + 1);
      ArrayResize(g_hCount, k + 1);
      ArrayResize(g_hBefore, k + 1);
      ArrayResize(g_hTries, k + 1);
      g_hSym[k] = JStr(line, "symbol");
      g_hTf[k] = JStr(line, "tf");
      g_hCount[k] = (int)JInt(line, "count");
      g_hBefore[k] = JInt(line, "before");
      g_hTries[k] = 0;
     }
   else if(t == "probe")
      Probe(line);
   else if(t == "delta")
     {
      int k = ArraySize(g_qSym);
      ArrayResize(g_qSym, k + 1);
      ArrayResize(g_qTf, k + 1);
      ArrayResize(g_qCount, k + 1);
      ArrayResize(g_qSkip, k + 1);
      ArrayResize(g_qRow, k + 1);
      g_qRow[k] = JNum(line, "row");
      g_qSym[k] = JStr(line, "symbol");
      g_qTf[k] = JStr(line, "tf");
      g_qCount[k] = (int)JInt(line, "count");
      g_qSkip[k] = (int)JInt(line, "skip");
     }
   else if(t == "objects")
      ProbeObjects(line);
   else
      SendError("comando desconhecido: " + t);
  }

void ReadCommands()
  {
   uint len = SocketIsReadable(g_sock);
   if(len > 0)
     {
      uchar tmp[];
      int r = SocketRead(g_sock, tmp, len, 10);
      if(r > 0)
        {
         int old = ArraySize(g_rx);
         ArrayResize(g_rx, old + r);
         ArrayCopy(g_rx, tmp, old, 0, r);
        }
     }
   int size = ArraySize(g_rx), start = 0;
   for(int i = 0; i < size && g_sock != INVALID_HANDLE; i++)
     {
      if(g_rx[i] != '\n')
         continue;
      if(i > start)
         Handle(CharArrayToString(g_rx, start, i - start, CP_UTF8));
      start = i + 1;
     }
   if(g_sock != INVALID_HANDLE && start > 0)
      ArrayRemove(g_rx, 0, start);
  }

//+------------------------------------------------------------------+
int OnInit()
  {
   if(InpTimerMs < 1 || !EventSetMillisecondTimer(InpTimerMs))
      return(INIT_PARAMETERS_INCORRECT);
   // Reinitialização por troca de conta: nunca reutiliza o realizado da conta anterior.
   g_dayStart = 0;
   g_dayRealized = 0.0;
   g_dayDirty = true;
   g_dayReady = false;
   g_nextDayRetry = 0;
   g_lastDaily = "";
   g_nextConnect = 0;
   return(INIT_SUCCEEDED);
  }

void OnDeinit(const int reason)
  {
   EventKillTimer();
   if(g_sock != INVALID_HANDLE)
      SocketClose(g_sock);
   g_sock = INVALID_HANDLE;
  }

void OnTimer()
  {
   if(g_sock == INVALID_HANDLE)
     {
      if(GetTickCount64() >= g_nextConnect)
         Connect();
      return;
     }
   if(!SocketIsConnected(g_sock))
     {
      Print("TerminalBridge: app desconectou");
      Disconnect();
      return;
     }
   ReadCommands();
   // histórico ainda não carregado no terminal: tenta cada pedido por ~10 s
   for(int i = 0; i < ArraySize(g_hSym) && g_sock != INVALID_HANDLE; i++)
     {
      bool done = SendHistory(g_hSym[i], g_hTf[i], g_hCount[i], g_hBefore[i]);
      if(!done && ++g_hTries[i] > 10000 / MathMax(InpTimerMs, 1))
        {
         if(g_hBefore[i] > 0) // nada mais antigo disponível: lista vazia encerra os pedidos do app
            Send(BarsHead(g_hSym[i], g_hTf[i], (int)SymbolInfoInteger(g_hSym[i], SYMBOL_DIGITS), g_hBefore[i]) + "]}\n");
         else
            SendError("histórico indisponível: " + g_hSym[i] + " " + g_hTf[i]);
         done = true;
        }
      if(done && g_sock != INVALID_HANDLE)
        {
         ArrayRemove(g_hSym, i, 1);
         ArrayRemove(g_hTf, i, 1);
         ArrayRemove(g_hCount, i, 1);
         ArrayRemove(g_hBefore, i, 1);
         ArrayRemove(g_hTries, i, 1);
         i--;
        }
     }
   if(g_sock != INVALID_HANDLE)
      PumpTicks();
   if(g_sock != INVALID_HANDLE && GetTickCount64() >= g_nextState)
     {
      g_nextState = GetTickCount64() + STATE_MS;
      SendState(false);
     }
   if(g_sock != INVALID_HANDLE && GetTickCount64() >= g_nextSymbol)
     {
      g_nextSymbol = GetTickCount64() + SYMBOL_MS;
      for(int i = 0; i < ArraySize(g_syms) && g_sock != INVALID_HANDLE; i++)
         SendSymbol(g_syms[i]);
     }
   // por último, e uma leitura por ciclo: ticks e ordens não esperam pelo delta
   if(g_sock != INVALID_HANDLE)
      DeltaStep();
  }

void OnTick()
  {
   // a cotação do gráfico sai na hora, sem esperar o timer
   if(g_sock != INVALID_HANDLE)
      PumpTicks();
  }

void OnTradeTransaction(const MqlTradeTransaction &trans, const MqlTradeRequest &request, const MqlTradeResult &result)
  {
   if(trans.type == TRADE_TRANSACTION_DEAL_ADD || trans.type == TRADE_TRANSACTION_DEAL_UPDATE ||
      trans.type == TRADE_TRANSACTION_DEAL_DELETE)
     {
      g_dayDirty = true;
      g_nextDayRetry = 0;
     }
   if(trans.type == TRADE_TRANSACTION_REQUEST)
     {
      for(int i = ArraySize(g_reqIds) - 1; i >= 0; i--)
        {
         if(g_reqIds[i] != result.request_id)
            continue;
         bool ok = OkRetcode(result.retcode);
         string msg = RetcodeText(result.retcode);
         if(!ok && result.comment != "")
            msg += ": " + result.comment;
         ulong ticket = result.deal != 0 ? result.deal : result.order;
         SendResult(g_cliIds[i], ok, result.retcode, msg, ticket, result.price);
         ArrayRemove(g_reqIds, i, 1);
         ArrayRemove(g_cliIds, i, 1);
         break;
        }
     }
   if(g_sock != INVALID_HANDLE)
      SendState(false);
  }
//+------------------------------------------------------------------+
