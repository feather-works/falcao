//! Sintaxe + escopos + bytecode de arquivos JS inteiros, medindo o tempo:
//!
//!     cargo run --release -p falcao-bytecode --example gerar -- arquivo.js pasta/ ...
//!
//! Com `FALCAO_DESMONTAR=1`, imprime o bytecode da função 0 de cada arquivo.

use falcao_bytecode::{compilar, desmontar::desmontar};
use falcao_sintaxe::{analisar_escopos, analisar_script};
use std::time::Instant;

fn main() {
    let mut arquivos = Vec::new();
    for a in std::env::args().skip(1) {
        let c = std::path::PathBuf::from(&a);
        if c.is_dir() {
            let mut d: Vec<_> = std::fs::read_dir(&c).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "js")).collect();
            d.sort();
            arquivos.extend(d);
        } else {
            arquivos.push(c);
        }
    }
    let (mut ok, mut bytes, mut total, mut codigo) = (0, 0, 0.0, 0usize);
    for c in &arquivos {
        let Ok(fonte) = std::fs::read_to_string(c) else { continue };
        let t = Instant::now();
        let r = analisar_script(&fonte).map_err(|e| (e.posicao, e.mensagem)).and_then(|(arv, lex)| {
            let esc = analisar_escopos(&arv);
            compilar(&arv, &esc, &lex).map_err(|e| (e.posicao, e.mensagem))
        });
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        match r {
            Ok(s) => {
                ok += 1;
                bytes += fonte.len();
                total += ms;
                codigo += s.funcoes.iter().map(|f| f.codigo.len()).sum::<usize>();
                if fonte.len() > 300_000 {
                    println!("{:>8.2} ms  {:>8} B  {} funções  {}", ms, fonte.len(), s.funcoes.len(), c.display());
                }
                if std::env::var_os("FALCAO_DESMONTAR").is_some() {
                    for f in &s.funcoes {
                        print!("{}", desmontar(&s, f));
                    }
                }
            }
            Err((pos, msg)) => {
                let i = pos as usize;
                let ini = fonte.floor_char_boundary(i.saturating_sub(40));
                let fim = fonte.ceil_char_boundary((i + 40).min(fonte.len()));
                println!("ERRO {}: {} em {}: …{}…", c.display(), msg, i, &fonte[ini..fim].replace('\n', "⏎"));
            }
        }
    }
    println!("{ok}/{} sem erro; {:.1} MB de fonte → {:.1} MB de bytecode em {:.1} ms", arquivos.len(), bytes as f64 / 1e6, codigo as f64 / 1e6, total);
}
