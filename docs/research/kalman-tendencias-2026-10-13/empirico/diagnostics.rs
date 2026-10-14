#![allow(dead_code, unused_imports)]
use std::{collections::{BTreeMap,BTreeSet},io::{BufWriter,Write}};
mod model {
    #[derive(Clone,Copy,Debug)]
    pub struct Bar { pub time:i64,pub open:f64,pub high:f64,pub low:f64,pub close:f64,pub volume:f64 }
}
#[path="source/ctx.rs"] mod ctx;
#[path="source/math.rs"] mod math;
#[path="source/trend.rs"] mod trend;
use model::Bar;
use trend::{Adaptive,KalmanParams,KalmanTrend,UP,DOWN,FLAT};
fn load(path:&str)->Vec<Bar> {
    let bars:Vec<_>=std::fs::read_to_string(path).unwrap().lines().skip(1).map(|l| {
        let x:Vec<f64>=l.split(',').map(|v|v.parse().unwrap()).collect();
        Bar{time:x[0] as i64,open:x[1],high:x[2],low:x[3],close:x[4],volume:x[5]}
    }).collect();
    assert!(bars.windows(2).all(|w|w[0].time<w[1].time)); bars
}
fn base()->KalmanParams {KalmanParams{window:500,level_noise:0.2805,slope_noise:0.01502,measurement_noise:3.26,
    adaptive:Adaptive::Efficiency,adapt_gain:0.5,adapt_span:200,adapt_cap:10.,slope_threshold:0.16,slope_exit:-0.92,..KalmanParams::default()}}
fn kernels()->Vec<(&'static str,KalmanParams)> {
    let b=base();let d=KalmanParams::default();
    vec![("baseline_eff500",b),("fixed500",KalmanParams{adaptive:Adaptive::Off,..b}),
        ("fixed50",KalmanParams{window:50,adaptive:Adaptive::Off,..b}),("default50",d),
        ("innovation_base_g2",KalmanParams{window:50,adaptive:Adaptive::Innovation,adapt_gain:2.,adapt_span:20,adapt_cap:4.,..b}),
        ("innovation_default_g4",KalmanParams{adaptive:Adaptive::Innovation,adapt_gain:4.,adapt_span:20,adapt_cap:16.,..d}),
        ("innovation_default_g2",KalmanParams{adaptive:Adaptive::Innovation,adapt_gain:2.,adapt_span:20,adapt_cap:4.,..d})]
}
fn colors(t:&[f64],entry:f64,exit:f64)->Vec<u8> {
    let mut c=vec![FLAT;t.len()];for i in 1..t.len()-1 {c[i]=match c[i-1] {
        UP if t[i]>exit=>UP,DOWN if t[i]< -exit=>DOWN,
        _=>if t[i]>entry {UP}else if t[i]< -entry {DOWN}else{FLAT}};} c
}
fn thresholds(name:&str,p:KalmanParams)->Vec<(f64,f64)> {
    let mut v=Vec::new();for e in [0.75,1.,1.5] {for x in [-0.92,0.,0.25,0.5] {v.push((e,x));}}
    if name=="baseline_eff500" || name=="default50" {v.push((p.slope_threshold,p.slope_exit));}v
}
fn original(bars:&[Bar],p:KalmanParams)->KalmanTrend {
    let mut out=KalmanTrend::unlimited(p);out.update(bars);out
}
fn map(chart:&[Bar],other:&[Bar],period:i64)->Vec<Option<usize>> {
    let mut j=0;chart.iter().map(|b| {while j<other.len() && other[j].time+period<=b.time+300 {j+=1;}
        if j>0 && j<other.len(){Some(j-1)}else{None}}).collect()
}
fn aligned(a:&[u8],b:&[u8],mb:&[Option<usize>],c:Option<(&[u8],&[Option<usize>])>)->Vec<u8> {
    a.iter().enumerate().map(|(i,&x)|if x!=FLAT && mb[i].is_some_and(|j|b[j]==x)
        && c.is_none_or(|(v,m)|m[i].is_some_and(|j|v[j]==x)){x}else{FLAT}).collect()
}
fn sign(x:u8)->f64 {if x==UP {1.}else if x==DOWN{-1.}else{0.}}
fn quantile(v:&mut Vec<f64>,q:f64)->f64 {
    if v.is_empty(){return f64::NAN;}v.sort_by(f64::total_cmp);v[((v.len()-1) as f64*q).round() as usize]
}
fn ratio(a:f64,b:f64)->f64 {if b>0.{a/b}else{f64::NAN}}
struct Context { atr:Vec<f64>,day:Vec<Option<i64>>,split:Vec<Option<usize>>,indices:[Vec<usize>;3],events:[Vec<(usize,u8)>;3] }
const SPLITS:[&str;3]=["2021_2022_reused","2023_2024_reused","2025_2026_reused"];
impl Context {
    fn new(bars:&[Bar])->Self {
        let n=bars.len();let mut atr=vec![f64::NAN;n];let(mut a,mut day,mut split)=(f64::NAN,vec![None;n],vec![None;n]);
        let mut indices:[Vec<usize>;3]=std::array::from_fn(|_|Vec::new());
        for i in 1..n-1 {
            let b=bars[i];let tr=(b.high-b.low).max((b.high-bars[i-1].close).abs()).max((b.low-bars[i-1].close).abs());
            a=if a.is_nan(){tr}else{a+(tr-a)/14.};if i>=14 {atr[i]=a;}
            let ny=ctx::ny(b.time);let d=ctx::day(ny);let m=(ny-d)/60;let y=ctx::ymd(d).0;
            if d>=ctx::date(2021,3,1) && d<ctx::date(2026,10,9) && m>=570 && m<ctx::session_close(d) {
                let s=if y<=2022{0}else if y<=2024{1}else{2};day[i]=Some(d);split[i]=Some(s);indices[s].push(i);
            }
        }
        let mut events:[Vec<(usize,u8)>;3]=std::array::from_fn(|_|Vec::new());
        let(mut last_side,mut candidate,mut consecutive,mut last_event,mut last_day)=(FLAT,FLAT,0,0,None);
        for i in 12..n-1 {
            if day[i].is_none() || day[i]!=last_day {last_side=FLAT;candidate=FLAT;consecutive=0;last_event=0;}last_day=day[i];
            if day[i].is_none() || day[i-12]!=day[i] || bars[i].time-bars[i-12].time!=3600 {continue;}
            let delta=(bars[i].close-bars[i-12].close)/atr[i];let side=if delta>=1.5{UP}else if delta<=-1.5{DOWN}else{FLAT};
            if side==candidate && side!=FLAT {consecutive+=1;}else{candidate=side;consecutive=1;}
            if side!=FLAT && consecutive>=3 && (last_event==0 || i>=last_event+12) && side!=last_side {
                if last_side!=FLAT {events[split[i].unwrap()].push((i,side));}last_side=side;last_event=i;
            }
        }
        // Retain only events with the complete 12-bar response horizon plus
        // two bars needed to establish persistence, avoiding EOD censoring.
        for ev in &mut events {ev.retain(|&(i,_)|i+14<n-1 && day[i+14]==day[i]
            && bars[i+14].time-bars[i].time==14*300);}
        Self{atr,day,split,indices,events}
    }
}
fn metrics(w:&mut BufWriter<std::fs::File>,symbol:&str,name:&str,e:f64,x:f64,view:&str,bars:&[Bar],k:&KalmanTrend,c:&[u8],cx:&Context) {
    for s in 0..3 {
        let idx=&cx.indices[s];let mut days=BTreeSet::new();let(mut changes,mut flips,mut colored,mut mismatch)=(0usize,0usize,0usize,0usize);
        let(mut runs,mut errors,mut speeds)=(Vec::new(),Vec::new(),Vec::new());let(mut prev,mut last_dir,mut run_len,mut run_day)=(FLAT,FLAT,0usize,None);
        for &i in idx {
            if !k.trend[i].is_finite(){continue;}let d=cx.day[i];days.insert(d);
            if d!=run_day || i==0 || cx.day[i-1]!=d {if run_len>0{runs.push(run_len as f64*5.);}prev=FLAT;last_dir=FLAT;run_len=0;}
            else {changes+=(c[i]!=prev) as usize;}
            if c[i]!=prev || d!=run_day {if run_len>0{runs.push(run_len as f64*5.);}run_len=0;}
            if c[i]!=FLAT {colored+=1;run_len+=1;mismatch+=(sign(c[i])*k.slope_t[i]<0.) as usize;
                if last_dir!=FLAT && c[i]!=last_dir {flips+=1;}last_dir=c[i];}
            errors.push((k.trend[i]-bars[i].close).abs()/cx.atr[i]);speeds.push(k.speed[i]);prev=c[i];run_day=d;
        }
        if run_len>0{runs.push(run_len as f64*5.);}
        let run_med=quantile(&mut runs,0.5);let err_med=quantile(&mut errors,0.5);let err_p90=quantile(&mut errors,0.9);let speed_med=quantile(&mut speeds,0.5);let speed_p10=quantile(&mut speeds,0.1);let speed_p90=quantile(&mut speeds,0.9);
        let mut delays=Vec::new();let mut misses=0;
        for &(i,side) in &cx.events[s] {
            let found=(0..=12).find(|&lag|i+lag+2<bars.len() && cx.day[i+lag+2]==cx.day[i] && (0..3).all(|j|c[i+lag+j]==side));
            if let Some(lag)=found {delays.push(lag as f64*5.);}else{misses+=1;}
        }
        let lag_med=quantile(&mut delays,0.5);let lag_p90=quantile(&mut delays,0.9);
        for h in [6usize,12,24] {
            let(mut eligible,mut col,mut correct,mut wrong)=(0usize,0usize,0usize,0usize);let(mut allmove,mut goodmove,mut badmove)=(0.,0.,0.);
            let(mut past_eligible,mut past_colored,mut past_correct)=(0usize,0usize,0usize);
            for &i in idx {
                if i>=h && cx.day[i-h]==cx.day[i] && bars[i].time-bars[i-h].time==h as i64*300 && k.trend[i].is_finite() {
                    let delta=(bars[i].close-bars[i-h].close)/cx.atr[i];
                    if delta.abs()>=1. {past_eligible+=1;if c[i]!=FLAT {past_colored+=1;past_correct+=(sign(c[i])*delta>0.) as usize;}}
                }
                if i+h>=bars.len()-1 || cx.day[i+h]!=cx.day[i] || bars[i+h].time-bars[i].time!=h as i64*300 || !k.trend[i].is_finite(){continue;}
                let delta=(bars[i+h].close-bars[i].close)/cx.atr[i];if delta.abs()<1.{continue;}
                eligible+=1;allmove+=delta.abs();if c[i]!=FLAT {col+=1;
                    if sign(c[i])*delta>0.{correct+=1;goodmove+=delta.abs();}else{wrong+=1;badmove+=delta.abs();}}
            }
            writeln!(w,"{symbol},{name},{e},{x},{view},{},{},{},{:.8},{:.8},{:.8},{run_med:.8},{:.8},{err_med:.8},{err_p90:.8},{speed_p10:.8},{speed_med:.8},{speed_p90:.8},{},{misses},{lag_med:.8},{lag_p90:.8},{},{eligible},{col},{correct},{wrong},{:.8},{:.8},{:.8},{:.8},{allmove},{goodmove},{badmove},{colored},{mismatch},{past_eligible},{past_colored},{past_correct}",
                SPLITS[s],days.len(),idx.len(),ratio(colored as f64,idx.len() as f64),ratio(changes as f64,days.len() as f64),ratio(flips as f64,days.len() as f64),ratio(mismatch as f64,colored as f64),cx.events[s].len(),h*5,
                ratio(correct as f64,col as f64),ratio(correct as f64,eligible as f64),ratio(goodmove,allmove),ratio(badmove,allmove)).unwrap();
        }
    }
}
fn checks(bars:&[Bar],p:KalmanParams) {
    let b=&bars[bars.len()-14000..bars.len()-2000];let full=original(b,p);assert_eq!(colors(&full.slope_t,p.slope_threshold,p.slope_exit),full.shade);
    let mut inc=KalmanTrend::unlimited(p);inc.update(&b[..10000]);for n in 10001..=b.len(){inc.update(&b[..n]);}
    for i in 0..b.len()-1{assert_eq!(inc.shade[i],full.shade[i]);if full.trend[i].is_finite(){assert_eq!(inc.trend[i],full.trend[i]);assert_eq!(inc.speed[i],full.speed[i]);}}
    let mut future=b.to_vec();let cut=9000;for bar in &mut future[cut..] {bar.open*=1.7;bar.high*=1.7;bar.low*=1.7;bar.close*=1.7;}
    let changed=original(&future,p);for i in 0..cut {assert_eq!(full.shade[i],changed.shade[i]);if full.trend[i].is_finite(){assert_eq!(full.trend[i],changed.trend[i]);assert_eq!(full.slope_t[i],changed.slope_t[i]);}}
    assert!(full.trend[b.len()-1].is_nan());
}
fn synthetic(segment_len:usize,drift:f64)->(Vec<Bar>,Vec<i32>) {
    let mut seed=20261013u64;let mut logp=100f64.ln();let mut bars=Vec::new();let mut regimes=Vec::new();
    for i in 0..10000 {let regime=if i<1000{0}else{match ((i-1000)/segment_len)%3{0=>1,1=>-1,_=>0}};
        seed^=seed<<13;seed^=seed>>7;seed^=seed<<17;let u=(seed as f64)/(u64::MAX as f64)*2.-1.;
        let noise=u*0.00030+if i%2==0{0.00010}else{-0.00010};let open=logp.exp();logp+=regime as f64*drift+noise;let close=logp.exp();
        bars.push(Bar{time:1_600_000_000+i as i64*300,open,high:open.max(close)*1.00020,low:open.min(close)*0.99980,close,volume:1.});regimes.push(regime);
    }(bars,regimes)
}
fn synthetic_metrics(w:&mut BufWriter<std::fs::File>,name:&str,p:KalmanParams) {
  for(scenario,len,drift_rate)in [("strong_long",500,0.00030),("weak_two_hour",24,0.00010)] {
    let(bars,regimes)=synthetic(len,drift_rate);let k=original(&bars,p);
    for(e,x)in thresholds(name,p){let c=colors(&k.slope_t,e,x);let(mut good,mut bad,mut drift,mut lat,mut lat_col,mut flips)=(0usize,0usize,0usize,0usize,0usize,0usize);
        let mut lag=Vec::new();let mut misses=0;let mut transitions=0;let mut lateral_last=FLAT;
        for i in 1000..bars.len()-1 {
            if regimes[i]!=0 {drift+=1;good+=(sign(c[i])==regimes[i] as f64) as usize;bad+=(sign(c[i])== -(regimes[i] as f64)) as usize;}
            else{lat+=1;lat_col+=(c[i]!=FLAT) as usize;
                if regimes[i-1]!=0{lateral_last=FLAT;}
                if c[i]!=FLAT{flips+=(lateral_last!=FLAT && c[i]!=lateral_last) as usize;lateral_last=c[i];}}
            if regimes[i]!=0 && regimes[i]!=regimes[i-1] {transitions+=1;let side=if regimes[i]>0{UP}else{DOWN};
                let horizon=60usize.min(len-3);
                let found=(0..=horizon).find(|&d|i+d+2<c.len()-1 && (0..3).all(|j|regimes[i+d+j]==regimes[i] && c[i+d+j]==side));
                if let Some(d)=found{lag.push(d as f64*5.);}else{misses+=1;}}
        }
        let med=quantile(&mut lag,0.5);let p90=quantile(&mut lag,0.9);
        writeln!(w,"{scenario},{name},{e},{x},{:.8},{:.8},{:.8},{:.8},{transitions},{misses},{med},{p90}",ratio(good as f64,drift as f64),ratio(bad as f64,drift as f64),ratio(lat_col as f64,lat as f64),100.*ratio(flips as f64,lat as f64)).unwrap();
    }
  }
}
fn main() {
    std::fs::create_dir_all("results").unwrap();let data=std::env::var("KALMAN_DATA").unwrap_or_else(|_|"/tmp/kalman-trend-data".into());
    let mut w=BufWriter::new(std::fs::File::create("results/metrics.csv").unwrap());
    writeln!(w,"symbol,kernel,entry,exit,view,split,days,rth_bars,occupancy,changes_per_session,flips_per_session,median_run_min,opposite_slope_fraction,level_median_atr,level_p90_atr,speed_p10,speed_median,speed_p90,reversals,reversal_misses,reversal_lag_median_min,reversal_lag_p90_min,horizon_min,labelled,colored,correct,wrong,direction_accuracy,correct_capture,magnitude_capture,opposite_magnitude_capture,magnitude_total,magnitude_correct,magnitude_wrong,state_colored,opposite_slope_count,past_labelled,past_colored,past_correct").unwrap();
    let mut sw=BufWriter::new(std::fs::File::create("results/synthetic.csv").unwrap());
    writeln!(sw,"scenario,kernel,entry,exit,correct_drift_occupancy,opposite_drift_occupancy,sideways_occupancy,sideways_flips_per_100,transitions,misses,lag_median_min,lag_p90_min").unwrap();
    let mut pw=BufWriter::new(std::fs::File::create("results/parameters.csv").unwrap());
    writeln!(pw,"kernel,entry,exit,window,level_noise,slope_noise,measurement_noise,adaptive,gain,span,cap").unwrap();
    for(name,p)in kernels(){synthetic_metrics(&mut sw,name,p);for(e,x)in thresholds(name,p){writeln!(pw,"{name},{e},{x},{},{},{},{},{:?},{},{},{}",p.window,p.level_noise,p.slope_noise,p.measurement_noise,p.adaptive,p.adapt_gain,p.adapt_span,p.adapt_cap).unwrap();}}
    for symbol in ["UsaTec","UsaInd"] {
        let bars=load(&format!("{data}/{symbol}_M5.csv"));let h1=load(&format!("{data}/{symbol}_H1.csv"));let m15=load(&format!("data/{symbol}_M15.csv"));
        let cx=Context::new(&bars);let mh=map(&bars,&h1,3600);let mm=map(&bars,&m15,900);
        for (i,&j) in mh.iter().enumerate(){if let Some(j)=j{assert!(h1[j].time+3600<=bars[i].time+300);}}
        for(name,p)in kernels(){checks(&bars,p);eprintln!("Checks OK {symbol} {name}; calculating original full history");
            let k=original(&bars,p);let kh=original(&h1,p);let km=original(&m15,p);
            assert_eq!(k.shade,colors(&k.slope_t,p.slope_threshold,p.slope_exit));
            for(e,x)in thresholds(name,p){let c=colors(&k.slope_t,e,x);let ch=colors(&kh.slope_t,e,x);let cm=colors(&km.slope_t,e,x);
                let a=aligned(&c,&ch,&mh,None);let b=aligned(&c,&ch,&mh,Some((&cm,&mm)));
                for(view,vc)in [("M5",&c),("M5_H1",&a),("M5_M15_H1",&b)]{metrics(&mut w,symbol,name,e,x,view,&bars,&k,vc,&cx);}
                let export=(name=="baseline_eff500" && e==0.16) || (e==1. && x==0.);
                if export {let suffix=if e==0.16{"baseline_exact"}else{name};
                    let mut ew=BufWriter::new(std::fs::File::create(format!("results/series_{symbol}_{suffix}.csv")).unwrap());
                    writeln!(ew,"server_time,ny_time,open,high,low,close,atr,level,slope_t,speed,M5,M15,H1,M5_H1,M5_M15_H1").unwrap();
                    for &i in &cx.indices[2]{let d=cx.day[i].unwrap();if d>=ctx::date(2026,10,5) && d<=ctx::date(2026,10,7) {
                        writeln!(ew,"{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",bars[i].time,ctx::ny(bars[i].time),bars[i].open,bars[i].high,bars[i].low,bars[i].close,cx.atr[i],k.trend[i],k.slope_t[i],k.speed[i],c[i],mm[i].map(|j|cm[j]).unwrap_or(FLAT),mh[i].map(|j|ch[j]).unwrap_or(FLAT),a[i],b[i]).unwrap();}}}
            }
        }
    }
    eprintln!("Done: original engine, incremental equivalence, future perturbation and closed-timeframe mapping verified for all seven kernels on both symbols.");
}
