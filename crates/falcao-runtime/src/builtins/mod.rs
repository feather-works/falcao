//! Biblioteca padrão. Cada arquivo instala um grupo de objetos no global.

mod array;
pub(crate) mod assincronos;
mod bigint;
pub(crate) mod colecoes;
mod data;
mod iterador;
mod json;
mod matematica;
mod nucleo;
mod numero;
mod objeto;
mod regexp;
mod texto;
mod tipados;

use crate::intrinsecos::simb;
use crate::funcoes::{Args, Res};
use crate::motor::Motor;
use crate::textos::ChaveProp;
use crate::valor::Valor;

pub(crate) fn instalar(m: &mut Motor) {
    nucleo::instalar(m);
    objeto::instalar(m);
    array::instalar(m);
    json::instalar(m);
    regexp::instalar(m);
    texto::instalar(m);
    numero::instalar(m);
    bigint::instalar(m);
    matematica::instalar(m);
    colecoes::instalar(m);
    iterador::instalar(m);
    data::instalar(m);
    tipados::instalar(m);
    m.instalar_promessas();
    assincronos::instalar(m);
    // Map, Set e Promise são instalados em arquivos de outras áreas; o getter entra aqui.
    for nome in ["Map", "Set", "Promise"] {
        let g = m.intr.global;
        let c = m.chave(nome);
        if let Ok(ctor) = m.obter(g, ChaveProp::Nome(c)) {
            ligar_especie(m, ctor);
        }
    }
}

/// `get [Symbol.species]` (retorna `this`) de um construtor, como §23.1.2.2 e afins.
pub(super) fn ligar_especie(m: &mut Motor, ctor: Valor) {
    let f = m.nativa("get [Symbol.species]", 0, especie, false);
    m.definir_acessor(ctor, ChaveProp::Nome(simb::SPECIES), f, false, false);
}

fn especie(_m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    Ok(this)
}
