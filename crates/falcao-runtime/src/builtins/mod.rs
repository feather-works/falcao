//! Biblioteca padrão. Cada arquivo instala um grupo de objetos no global.

mod array;
mod bigint;
pub(crate) mod colecoes;
mod data;
mod json;
mod matematica;
mod nucleo;
mod numero;
mod objeto;
mod regexp;
mod texto;
mod tipados;

use crate::motor::Motor;

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
    data::instalar(m);
    tipados::instalar(m);
    m.instalar_promessas();
}
