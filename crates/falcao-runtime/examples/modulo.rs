//! Roda um módulo ES do disco (e o que ele importar): `cargo run --release --example modulo -- a.mjs`.
//! Nomes soltos (`'preact'`) vêm de `FALCAO_MAPA="preact=/caminho/preact.mjs,..."` (mapa de importação).

use falcao_runtime::modulos::CarregadorModulos;
use falcao_runtime::Motor;
use std::time::Instant;

struct Disco;

impl CarregadorModulos for Disco {
    fn baixar(&mut self, url: &str) -> Result<String, String> {
        let caminho = url.strip_prefix("file://").ok_or("só file://")?;
        std::fs::read_to_string(caminho).map_err(|e| e.to_string())
    }

    fn resolver(&mut self, base: &str, especificador: &str) -> Result<String, String> {
        let mapa = std::env::var("FALCAO_MAPA").unwrap_or_default();
        match mapa.split(',').filter_map(|x| x.split_once('=')).find(|(n, _)| *n == especificador) {
            Some((_, c)) => Ok(format!("file://{c}")),
            None => falcao_runtime::modulos::juntar_url(base, especificador),
        }
    }
}

fn main() {
    let caminho = std::env::args().nth(1).expect("uso: modulo arquivo.mjs");
    let caminho = std::fs::canonicalize(caminho).expect("arquivo não existe");
    let fonte = std::fs::read_to_string(&caminho).unwrap();
    let mut m = Motor::novo();
    m.carregador_modulos = Some(Box::new(Disco));
    let t = Instant::now();
    if let Err(e) = m.avaliar_modulo(&format!("file://{}", caminho.display()), &fonte) {
        m.relatar_erro(e);
    }
    m.rodar_microtarefas();
    for e in m.tirar_rejeicoes_nao_tratadas() {
        m.relatar_erro(e);
    }
    eprintln!("[{} módulos, {:.1} ms, coletas {:?}]", m.modulos.len(), t.elapsed().as_secs_f64() * 1000.0, m.coletas);
}
