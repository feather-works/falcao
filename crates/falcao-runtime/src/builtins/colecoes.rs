//! Map, Set, WeakMap, WeakSet, WeakRef e FinalizationRegistry.
//!
//! As entradas ficam numa `Tabela` dentro de `Motor::tabelas`; o objeto guarda o índice dela no
//! campo interno 1 (cru). Entrada apagada vira `None`, então um iterador em andamento continua
//! certo.
//!
//! O coletor (`coletor.rs`) percorre `Motor::tabelas` e refaz as chaves `Ref` (que guardam
//! `Valor.0`) quando os objetos mudam de lugar; libera a tabela de mapa morto e tira de
//! WeakMap/WeakSet as entradas de chave morta.
//!
//! shortcut: o índice da tabela liberada não é reaproveitado e `clear` deixa `None` no lugar;
//! upgrade: lista de tabelas livres e compactar os itens quando `vivos` for pequeno.
//! shortcut: WeakRef segura o alvo como propriedade normal; FinalizationRegistry guarda nada e
//! o callback nunca roda.

use std::collections::HashMap;

use crate::formas::{attr, Chave, Classe};
use crate::funcoes::{Args, FnNativa, Res};
use crate::intrinsecos::simb;
use crate::motor::Motor;
use crate::nomes::fixos;
use crate::textos::{ChaveProp, Unidades};
use crate::valor::Valor;

/// Chave de uma entrada pela igualdade SameValueZero.
#[derive(Hash, PartialEq, Eq, Clone)]
pub enum ChaveTabela {
    /// Bits do f64; `-0` vira `+0` e todo NaN vira o mesmo.
    Num(u64),
    Texto(Vec<u16>),
    /// BigInt pelo valor.
    BigInt(crate::grande::Grande),
    /// `Valor.0` de objetos, símbolos e dos fixos.
    Ref(u32),
}

/// Entradas de um Map/Set, em ordem de inserção. Apagadas ficam `None`.
pub struct Tabela {
    pub itens: Vec<Option<(Valor, Valor)>>,
    /// Números, BigInt, objetos e textos com unidade acima de 255.
    pub indice: HashMap<ChaveTabela, usize>,
    /// Textos só com unidades até 255 (quase todas as chaves), pelos bytes: a busca lê os bytes
    /// direto da gaiola, sem copiar o texto a cada `get`/`has`.
    pub textos: HashMap<Bytes, usize>,
    pub vivos: usize,
}

impl Tabela {
    pub(crate) fn nova() -> Self {
        Tabela { itens: Vec::new(), indice: HashMap::new(), textos: HashMap::new(), vivos: 0 }
    }
}

/// Bytes de uma chave de texto. As curtas (quase todas) moram na própria entrada da tabela:
/// a busca não segue um ponteiro (uma falta de cache a menos) e nada vai para o `malloc`.
#[derive(Clone)]
pub enum Bytes {
    Curta(u8, [u8; CURTA]),
    Longa(Box<[u8]>),
}

const CURTA: usize = 22;

impl Bytes {
    fn de(b: &[u8]) -> Bytes {
        if b.len() <= CURTA {
            let mut a = [0; CURTA];
            a[..b.len()].copy_from_slice(b);
            Bytes::Curta(b.len() as u8, a)
        } else {
            Bytes::Longa(b.into())
        }
    }
}

impl std::borrow::Borrow<[u8]> for Bytes {
    fn borrow(&self) -> &[u8] {
        match self {
            Bytes::Curta(n, a) => &a[..*n as usize],
            Bytes::Longa(b) => b,
        }
    }
}

// Hash e igualdade pelos bytes, iguais aos de `[u8]`: a busca usa a fatia emprestada.
impl std::hash::Hash for Bytes {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        std::borrow::Borrow::<[u8]>::borrow(self).hash(h)
    }
}

impl PartialEq for Bytes {
    fn eq(&self, o: &Bytes) -> bool {
        std::borrow::Borrow::<[u8]>::borrow(self) == std::borrow::Borrow::<[u8]>::borrow(o)
    }
}

impl Eq for Bytes {}

/// Chave dona (para inserir ou apagar).
enum Dona {
    Bytes(Bytes),
    Outra(ChaveTabela),
}

/// Unidades de texto até 255 como bytes; `None` se alguma passa (fica em `indice`).
fn latin1(u: &[u16]) -> Option<Bytes> {
    let b: Option<Vec<u8>> = u.iter().map(|&c| u8::try_from(c).ok()).collect();
    b.map(|b| Bytes::de(&b))
}

fn chave_dona(m: &mut Motor, k: Valor) -> Dona {
    if m.e_texto(k) {
        let p = m.achatar(k);
        return match m.unidades_planas(p) {
            Unidades::Latin1(b) => Dona::Bytes(Bytes::de(b)),
            Unidades::Utf16(u) => match latin1(u) {
                Some(b) => Dona::Bytes(b),
                None => Dona::Outra(ChaveTabela::Texto(u.to_vec())),
            },
        };
    }
    Dona::Outra(chave_tabela(m, k))
}

/// Posição da chave `k` na tabela, sem alocar quando ela é texto Latin1 já plano.
fn achar(m: &mut Motor, idx: usize, k: Valor) -> Option<usize> {
    if m.e_texto(k) {
        let p = m.achatar(k);
        if let Unidades::Latin1(b) = m.unidades_planas(p) {
            return m.tabelas[idx].textos.get(b).copied();
        }
    }
    match chave_dona(m, k) {
        Dona::Bytes(b) => m.tabelas[idx].textos.get(&b).copied(),
        Dona::Outra(ch) => m.tabelas[idx].indice.get(&ch).copied(),
    }
}

const IT_CHAVES: u32 = 0;
const IT_VALORES: u32 = 1;
const IT_PARES: u32 = 2;

pub fn chave_tabela(m: &mut Motor, v: Valor) -> ChaveTabela {
    if m.e_numero(v) {
        let x = m.f64_de(v);
        let x = if x == 0.0 {
            0.0
        } else if x.is_nan() {
            f64::NAN
        } else {
            x
        };
        return ChaveTabela::Num(x.to_bits());
    }
    if m.e_texto(v) {
        return ChaveTabela::Texto(m.texto_u16_de(v));
    }
    if m.e_bigint(v) {
        return ChaveTabela::BigInt(m.ler_bigint(v));
    }
    ChaveTabela::Ref(v.0)
}

pub(super) fn instalar(m: &mut Motor) {
    let op = m.intr.objeto_proto;
    let ip = m.intr.iterador_proto;
    let mip = proto_com(m, ip);
    let sip = proto_com(m, ip);

    // ---- Map
    let mp = proto_com(m, op);
    let mapa = m.nativa("Map", 0, map_construtor, true);
    ligar(m, mapa, "Map", mp);
    metodo_cap(m, mapa, "groupBy", 2, map_group_by, mp);
    m.metodo(mp, "get", 1, map_get);
    m.metodo(mp, "set", 2, map_set);
    m.metodo(mp, "has", 1, map_has);
    m.metodo(mp, "delete", 1, map_delete);
    m.metodo(mp, "clear", 0, map_clear);
    m.metodo(mp, "forEach", 1, map_for_each);
    getter_tamanho(m, mp, map_size);
    metodo_cap(m, mp, "keys", 0, map_keys, mip);
    metodo_cap(m, mp, "values", 0, map_values, mip);
    let ent = metodo_cap(m, mp, "entries", 0, map_entries, mip);
    m.definir_propria(mp, simb::ITERATOR, ent, attr::METODO);
    etiqueta(m, mp, "Map");

    // ---- Set
    let sp = proto_com(m, op);
    let set = m.nativa("Set", 0, set_construtor, true);
    ligar(m, set, "Set", sp);
    m.metodo(sp, "add", 1, set_add);
    m.metodo(sp, "has", 1, set_has);
    m.metodo(sp, "delete", 1, set_delete);
    m.metodo(sp, "clear", 0, set_clear);
    m.metodo(sp, "forEach", 1, set_for_each);
    getter_tamanho(m, sp, set_size);
    let vals = metodo_cap(m, sp, "values", 0, set_values, sip);
    m.definir_propria(sp, fixos::KEYS, vals, attr::METODO);
    metodo_cap(m, sp, "entries", 0, set_entries, sip);
    m.definir_propria(sp, simb::ITERATOR, vals, attr::METODO);
    m.metodo(sp, "union", 1, set_union);
    m.metodo(sp, "intersection", 1, set_intersection);
    m.metodo(sp, "difference", 1, set_difference);
    m.metodo(sp, "symmetricDifference", 1, set_sym_difference);
    m.metodo(sp, "isSubsetOf", 1, set_is_subset_of);
    m.metodo(sp, "isSupersetOf", 1, set_is_superset_of);
    m.metodo(sp, "isDisjointFrom", 1, set_is_disjoint_from);
    etiqueta(m, sp, "Set");

    // ---- iteradores de Map e Set (o `next` é o mesmo: os dois têm a tabela no alvo)
    m.metodo(mip, "next", 0, iterador_next);
    m.metodo(sip, "next", 0, iterador_next);
    etiqueta(m, mip, "Map Iterator");
    etiqueta(m, sip, "Set Iterator");

    // ---- WeakMap
    let wmp = proto_com(m, op);
    let wm = m.nativa("WeakMap", 0, wm_construtor, true);
    ligar(m, wm, "WeakMap", wmp);
    m.metodo(wmp, "get", 1, wm_get);
    m.metodo(wmp, "set", 2, wm_set);
    m.metodo(wmp, "has", 1, wm_has);
    m.metodo(wmp, "delete", 1, wm_delete);
    etiqueta(m, wmp, "WeakMap");

    // ---- WeakSet
    let wsp = proto_com(m, op);
    let ws = m.nativa("WeakSet", 0, ws_construtor, true);
    ligar(m, ws, "WeakSet", wsp);
    m.metodo(wsp, "add", 1, ws_add);
    m.metodo(wsp, "has", 1, ws_has);
    m.metodo(wsp, "delete", 1, ws_delete);
    etiqueta(m, wsp, "WeakSet");

    // ---- WeakRef: o alvo fica numa propriedade com chave-símbolo escondida. O símbolo
    // nasce aqui e chega às funções como valor capturado (não há campo próprio no motor).
    let segredo = m.novo_simbolo(Valor::UNDEFINED);
    let k = m.chave_do_simbolo(segredo);
    m.escondidos.insert(k.0 & !Chave::BIT_SIMBOLO);
    let wrp = proto_com(m, op);
    let id = m.registrar_nativa("WeakRef", weakref_construtor, true);
    let wr = m.nativa_com(id, 1, segredo);
    ligar(m, wr, "WeakRef", wrp);
    metodo_cap(m, wrp, "deref", 0, weakref_deref, segredo);
    etiqueta(m, wrp, "WeakRef");

    // ---- FinalizationRegistry (shortcut: nada é guardado)
    let frp = proto_com(m, op);
    let fr = m.nativa("FinalizationRegistry", 1, fr_construtor, true);
    ligar(m, fr, "FinalizationRegistry", frp);
    m.metodo(frp, "register", 2, fr_register);
    m.metodo(frp, "unregister", 1, fr_unregister);
    etiqueta(m, frp, "FinalizationRegistry");

}

// ---------------------------------------------------------------- auxiliares de instalação

fn proto_com(m: &mut Motor, pai: Valor) -> Valor {
    let f = m.forma_para(Classe::Comum, pai, 0);
    m.novo_objeto(f)
}

/// Liga o construtor ao protótipo e ao global.
fn ligar(m: &mut Motor, c: Valor, nome: &str, proto: Valor) {
    m.definir_propria(c, fixos::PROTOTYPE, proto, 0);
    m.definir_propria(proto, fixos::CONSTRUCTOR, c, attr::METODO);
    let k = m.chave(nome);
    let g = m.intr.global;
    m.definir_propria(g, k, c, attr::METODO);
}

/// Método nativo com um valor capturado (protótipo do iterador, ou o segredo do WeakRef).
fn metodo_cap(m: &mut Motor, obj: Valor, nome: &'static str, comp: u32, f: FnNativa, cap: Valor) -> Valor {
    let id = m.registrar_nativa(nome, f, false);
    let fun = m.nativa_com(id, comp, cap);
    let c = m.chave(nome);
    m.definir_propria(obj, c, fun, attr::METODO);
    fun
}

fn getter_tamanho(m: &mut Motor, obj: Valor, f: FnNativa) {
    let g = m.nativa("get size", 0, f, false);
    m.definir_acessor(obj, ChaveProp::Nome(fixos::SIZE), g, false, false);
}

fn etiqueta(m: &mut Motor, obj: Valor, nome: &str) {
    let t = m.texto(nome);
    m.definir_propria(obj, simb::TO_STRING_TAG, t, attr::CONFIGURAVEL);
}

// ---------------------------------------------------------------- tabelas

/// Chave `-0` vira `+0` (também na entrada guardada).
fn sem_menos_zero(m: &Motor, k: Valor) -> Valor {
    if m.e_numero(k) && m.f64_de(k) == 0.0 {
        Valor::int(0)
    } else {
        k
    }
}

fn inserir(m: &mut Motor, idx: usize, k: Valor, v: Valor) {
    let k = sem_menos_zero(m, k);
    let dona = chave_dona(m, k);
    let t = &mut m.tabelas[idx];
    let novo = t.itens.len();
    // Uma busca só: achou, troca o valor; não achou, a entrada já fica reservada.
    let p = *match dona {
        Dona::Bytes(b) => t.textos.entry(b).or_insert(novo),
        Dona::Outra(ch) => t.indice.entry(ch).or_insert(novo),
    };
    if p == novo {
        t.itens.push(Some((k, v)));
        t.vivos += 1;
    } else {
        t.itens[p] = Some((k, v));
    }
}

fn ler(m: &mut Motor, idx: usize, k: Valor) -> Option<(Valor, Valor)> {
    let p = achar(m, idx, k)?;
    m.tabelas[idx].itens[p]
}

fn contem(m: &mut Motor, idx: usize, k: Valor) -> bool {
    ler(m, idx, k).is_some()
}

fn apagar(m: &mut Motor, idx: usize, k: Valor) -> bool {
    let dona = chave_dona(m, k);
    let t = &mut m.tabelas[idx];
    let r = match dona {
        Dona::Bytes(b) => t.textos.remove(&b),
        Dona::Outra(ch) => t.indice.remove(&ch),
    };
    match r {
        Some(p) => {
            t.itens[p] = None;
            t.vivos -= 1;
            true
        }
        None => false,
    }
}

fn limpar(m: &mut Motor, idx: usize) {
    let t = &mut m.tabelas[idx];
    for x in t.itens.iter_mut() {
        *x = None;
    }
    t.indice.clear();
    t.textos.clear();
    t.vivos = 0;
}

fn chaves_vivas(m: &Motor, idx: usize) -> Vec<Valor> {
    m.tabelas[idx].itens.iter().flatten().map(|&(k, _)| k).collect()
}

/// Objeto novo com tabela própria (índice no interno 1).
fn novo_com(m: &mut Motor, proto: Valor, classe: Classe) -> Valor {
    let f = m.forma_para(classe, proto, 0);
    let o = m.novo_objeto(f);
    let idx = m.tabelas.len();
    m.tabelas.push(Tabela::nova());
    m.por_interno(o, 1, idx as u32);
    o
}

/// Protótipo de `new X()`: o `prototype` de `nt`, ou o do próprio construtor.
fn proto_de_nt(m: &mut Motor, nt: Valor) -> Res<Valor> {
    let p = m.obter(nt, ChaveProp::Nome(fixos::PROTOTYPE))?;
    if m.e_objeto(p) {
        return Ok(p);
    }
    let padrao = m.chamado;
    m.obter(padrao, ChaveProp::Nome(fixos::PROTOTYPE))
}

fn exigir_new(m: &mut Motor, nt: Valor, nome: &str) -> Res<()> {
    if nt == Valor::UNDEFINED {
        return m.erro_tipo(&format!("Constructor {nome} requires 'new'"));
    }
    Ok(())
}

/// Índice da tabela do receptor, ou TypeError se o objeto não é de uma das classes.
fn tabela_de(m: &mut Motor, this: Valor, classes: &[Classe], nome: &str) -> Res<usize> {
    if m.e_objeto(this) {
        let c = m.formas.get(m.forma(this)).classe;
        if classes.contains(&c) {
            return Ok(m.interno(this, 1) as usize);
        }
    }
    m.erro_tipo(&format!("Method {nome} called on incompatible receiver"))
}

/// `adder` é a nativa `f` original (não foi trocada)?
fn e_nativa(m: &Motor, adder: Valor, f: FnNativa) -> bool {
    use crate::funcoes::{BIT_NATIVA, F_CODIGO};
    if !m.e_objeto(adder) || m.formas.get(m.forma(adder)).classe != Classe::Funcao {
        return false;
    }
    let c = m.interno(adder, F_CODIGO);
    c & BIT_NATIVA != 0 && std::ptr::fn_addr_eq(m.nativas[(c & !BIT_NATIVA) as usize].f, f)
}

/// AddEntriesFromIterable / o laço de `new Set(it)`: chama o `set`/`add` atual do objeto a cada
/// item (`par`: item `[k, v]`) e fecha o iterador se algo lançar. Com o método original, insere
/// direto (o mesmo efeito, sem a chamada).
fn adicionar_de(m: &mut Motor, o: Valor, it: Valor, metodo: &str, original: FnNativa, par: bool, fraca: Option<&str>) -> Res<()> {
    let k = m.chave(metodo);
    let adder = m.obter(o, ChaveProp::Nome(k))?;
    if !m.e_funcao(adder) {
        return m.erro_tipo(&format!("'{metodo}' returned for property '{metodo}' of object is not a function"));
    }
    let direto = e_nativa(m, adder, original);
    let idx = m.interno(o, 1) as usize;
    let reg = m.obter_iterador(it)?;
    while let Some(x) = m.proximo(reg)? {
        let r = (|| {
            let (k, v) = if par { par_de(m, x)? } else { (x, x) };
            if !direto {
                let args: &[Valor] = if par { &[k, v] } else { &[k] };
                return m.chamar(adder, o, args).map(drop);
            }
            if let Some(msg) = fraca {
                validar_fraca(m, k, msg)?;
            }
            inserir(m, idx, k, v);
            Ok(())
        })();
        if let Err(e) = r {
            // O erro original vence o de `return`.
            let _ = m.fechar_iterador(reg);
            return Err(e);
        }
    }
    Ok(())
}

/// Item `[k, v]` de `new Map(iterável)`.
fn par_de(m: &mut Motor, par: Valor) -> Res<(Valor, Valor)> {
    if !m.e_objeto(par) {
        let d = m.inspecionar(par, 0);
        return m.erro_tipo(&format!("Iterator value {d} is not an entry object"));
    }
    let k = m.obter(par, ChaveProp::Indice(0))?;
    let v = m.obter(par, ChaveProp::Indice(1))?;
    Ok((k, v))
}

/// Chave que pode ir num WeakMap/WeakSet: objeto, ou símbolo não registrado.
fn e_fraco(m: &Motor, v: Valor) -> bool {
    m.e_objeto(v) || (m.e_simbolo(v) && !m.registro_simbolos.values().any(|&s| s == v))
}

fn validar_fraca(m: &mut Motor, k: Valor, msg: &str) -> Res<()> {
    if e_fraco(m, k) {
        Ok(())
    } else {
        m.erro_tipo(msg)
    }
}

fn tamanho(m: &mut Motor, this: Valor, classes: &[Classe], nome: &str) -> Res<Valor> {
    let idx = tabela_de(m, this, classes, nome)?;
    let n = m.tabelas[idx].vivos;
    Ok(m.numero(n as f64))
}

/// `forEach`: o laço usa posição, então entradas acrescentadas no meio são visitadas.
fn para_cada(m: &mut Motor, idx: usize, cb: Valor, arg_this: Valor, obj: Valor, mapa: bool) -> Res<()> {
    if !m.e_funcao(cb) {
        let d = m.inspecionar(cb, 0);
        return m.erro_tipo(&format!("{d} is not a function"));
    }
    // O coletor roda no callback: o callback, o `this` dele e o objeto moram em `raizes` (os
    // itens, na tabela, que é raiz; o índice dela não muda).
    let base = m.raizes.len();
    m.raizes.extend([cb, arg_this, obj]);
    let mut i = 0;
    let r = loop {
        let item = match m.tabelas[idx].itens.get(i) {
            Some(x) => *x,
            None => break Ok(()),
        };
        i += 1;
        if let Some((k, v)) = item {
            let (cb, arg_this, obj) = (m.raizes[base], m.raizes[base + 1], m.raizes[base + 2]);
            let args = if mapa { [v, k, obj] } else { [k, k, obj] };
            if let Err(e) = m.chamar_coletavel(cb, arg_this, &args) {
                break Err(e);
            }
        }
    };
    m.raizes.truncate(base);
    r
}

fn novo_iterador(m: &mut Motor, this: Valor, classes: &[Classe], nome: &str, tipo: u32, proto: Valor) -> Res<Valor> {
    tabela_de(m, this, classes, nome)?;
    let f = m.forma_para(Classe::Iterador, proto, 0);
    let it = m.novo_objeto(f);
    m.por_interno_valor(it, 0, this);
    m.por_interno(it, 1, 0);
    m.por_interno(it, 2, tipo);
    Ok(it)
}

/// `next` de Map e Set: alvo no interno 0, posição no 1, tipo no 2.
fn iterador_next(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    if !m.e_objeto(this) || m.formas.get(m.forma(this)).classe != Classe::Iterador {
        return m.erro_tipo("next method called on incompatible receiver");
    }
    let alvo = Valor(m.interno(this, 0));
    if alvo == Valor::UNDEFINED {
        return Ok(m.resultado_iterador(Valor::UNDEFINED, true));
    }
    let idx = m.interno(alvo, 1) as usize;
    let tipo = m.interno(this, 2);
    let mut i = m.interno(this, 1) as usize;
    let len = m.tabelas[idx].itens.len();
    while i < len {
        let item = m.tabelas[idx].itens[i];
        i += 1;
        if let Some((k, v)) = item {
            m.por_interno(this, 1, i as u32);
            let r = match tipo {
                IT_CHAVES => k,
                IT_VALORES => v,
                _ => m.array_de(&[k, v]),
            };
            return Ok(m.resultado_iterador(r, false));
        }
    }
    m.por_interno(this, 1, i as u32);
    m.por_interno_valor(this, 0, Valor::UNDEFINED);
    Ok(m.resultado_iterador(Valor::UNDEFINED, true))
}

// ---------------------------------------------------------------- Map

fn map_construtor(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    exigir_new(m, nt, "Map")?;
    let p = proto_de_nt(m, nt)?;
    let o = novo_com(m, p, Classe::Mapa);
    let it = m.arg(a, 0);
    if !it.e_nulo_ou_indefinido() {
        adicionar_de(m, o, it, "set", map_set, true, None)?;
    }
    Ok(o)
}

fn map_get(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let idx = tabela_de(m, this, &[Classe::Mapa], "Map.prototype.get")?;
    let k = m.arg(a, 0);
    Ok(ler(m, idx, k).map_or(Valor::UNDEFINED, |(_, v)| v))
}

fn map_set(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let idx = tabela_de(m, this, &[Classe::Mapa], "Map.prototype.set")?;
    let (k, v) = (m.arg(a, 0), m.arg(a, 1));
    inserir(m, idx, k, v);
    Ok(this)
}

fn map_has(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let idx = tabela_de(m, this, &[Classe::Mapa], "Map.prototype.has")?;
    let k = m.arg(a, 0);
    Ok(Valor::bool(contem(m, idx, k)))
}

fn map_delete(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let idx = tabela_de(m, this, &[Classe::Mapa], "Map.prototype.delete")?;
    let k = m.arg(a, 0);
    Ok(Valor::bool(apagar(m, idx, k)))
}

fn map_clear(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let idx = tabela_de(m, this, &[Classe::Mapa], "Map.prototype.clear")?;
    limpar(m, idx);
    Ok(Valor::UNDEFINED)
}

fn map_for_each(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let idx = tabela_de(m, this, &[Classe::Mapa], "Map.prototype.forEach")?;
    let (cb, arg_this) = (m.arg(a, 0), m.arg(a, 1));
    para_cada(m, idx, cb, arg_this, this, true)?;
    Ok(Valor::UNDEFINED)
}

fn map_size(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    tamanho(m, this, &[Classe::Mapa], "get Map.prototype.size")
}

fn map_keys(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let p = m.capturado();
    novo_iterador(m, this, &[Classe::Mapa], "Map.prototype.keys", IT_CHAVES, p)
}

fn map_values(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let p = m.capturado();
    novo_iterador(m, this, &[Classe::Mapa], "Map.prototype.values", IT_VALORES, p)
}

fn map_entries(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let p = m.capturado();
    novo_iterador(m, this, &[Classe::Mapa], "Map.prototype.entries", IT_PARES, p)
}

/// `Map.groupBy(itens, cb)`: agrupa em arrays pela chave devolvida por `cb`.
fn map_group_by(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let p = m.capturado();
    let itens_v = m.arg(a, 0);
    if itens_v.e_nulo_ou_indefinido() {
        return m.erro_tipo("Map.groupBy called on null or undefined");
    }
    let cb = m.arg(a, 1);
    if !m.e_funcao(cb) {
        let d = m.inspecionar(cb, 0);
        return m.erro_tipo(&format!("{d} is not a function"));
    }
    let mut grupos: Vec<(Valor, Vec<Valor>)> = Vec::new();
    let mut pos: HashMap<ChaveTabela, usize> = HashMap::new();
    m.percorrer_iteravel(itens_v, |m, x, i| {
        let iv = m.numero(i as f64);
        let k = m.chamar(cb, Valor::UNDEFINED, &[x, iv])?;
        let k = sem_menos_zero(m, k);
        let ch = chave_tabela(m, k);
        match pos.get(&ch).copied() {
            Some(g) => grupos[g].1.push(x),
            None => {
                pos.insert(ch, grupos.len());
                grupos.push((k, vec![x]));
            }
        }
        Ok(())
    })?;
    let o = novo_com(m, p, Classe::Mapa);
    let idx = m.interno(o, 1) as usize;
    for (k, grupo) in grupos {
        let arr = m.array_de(&grupo);
        inserir(m, idx, k, arr);
    }
    Ok(o)
}

// ---------------------------------------------------------------- Set

fn set_construtor(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    exigir_new(m, nt, "Set")?;
    let p = proto_de_nt(m, nt)?;
    let o = novo_com(m, p, Classe::Conjunto);
    let it = m.arg(a, 0);
    if !it.e_nulo_ou_indefinido() {
        adicionar_de(m, o, it, "add", set_add, false, None)?;
    }
    Ok(o)
}

fn set_add(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let idx = tabela_de(m, this, &[Classe::Conjunto], "Set.prototype.add")?;
    let x = m.arg(a, 0);
    inserir(m, idx, x, x);
    Ok(this)
}

fn set_has(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let idx = tabela_de(m, this, &[Classe::Conjunto], "Set.prototype.has")?;
    let x = m.arg(a, 0);
    Ok(Valor::bool(contem(m, idx, x)))
}

fn set_delete(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let idx = tabela_de(m, this, &[Classe::Conjunto], "Set.prototype.delete")?;
    let x = m.arg(a, 0);
    Ok(Valor::bool(apagar(m, idx, x)))
}

fn set_clear(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let idx = tabela_de(m, this, &[Classe::Conjunto], "Set.prototype.clear")?;
    limpar(m, idx);
    Ok(Valor::UNDEFINED)
}

fn set_for_each(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let idx = tabela_de(m, this, &[Classe::Conjunto], "Set.prototype.forEach")?;
    let (cb, arg_this) = (m.arg(a, 0), m.arg(a, 1));
    para_cada(m, idx, cb, arg_this, this, false)?;
    Ok(Valor::UNDEFINED)
}

fn set_size(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    tamanho(m, this, &[Classe::Conjunto], "get Set.prototype.size")
}

fn set_values(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let p = m.capturado();
    novo_iterador(m, this, &[Classe::Conjunto], "Set.prototype.values", IT_VALORES, p)
}

fn set_entries(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let p = m.capturado();
    novo_iterador(m, this, &[Classe::Conjunto], "Set.prototype.entries", IT_PARES, p)
}

/// Operações entre conjuntos. shortcut: o argumento tem de ser um Set (nada de set-like).
#[derive(Clone, Copy)]
enum Op {
    Uniao,
    Intersecao,
    Diferenca,
    DiferencaSimetrica,
    ESubconjunto,
    ESuperconjunto,
    ESdisjunto,
}

fn outro_conjunto(m: &mut Motor, v: Valor) -> Res<usize> {
    if m.e_objeto(v) && m.formas.get(m.forma(v)).classe == Classe::Conjunto {
        return Ok(m.interno(v, 1) as usize);
    }
    m.erro_tipo("argument must be a Set")
}

/// Os itens de `xs` cuja presença na tabela `idx` é `presente`.
fn filtrar(m: &mut Motor, xs: &[Valor], idx: usize, presente: bool) -> Vec<Valor> {
    let mut r = Vec::new();
    for &x in xs {
        if contem(m, idx, x) == presente {
            r.push(x);
        }
    }
    r
}

/// Conjunto novo (com o protótipo de `this`) com os itens dados.
fn criar_conjunto(m: &mut Motor, this: Valor, itens: &[Valor]) -> Valor {
    let p = m.proto_de(this);
    let o = novo_com(m, p, Classe::Conjunto);
    let idx = m.interno(o, 1) as usize;
    for &x in itens {
        inserir(m, idx, x, x);
    }
    o
}

fn conjuntos_op(m: &mut Motor, this: Valor, a: Args, op: Op) -> Res<Valor> {
    let idx = tabela_de(m, this, &[Classe::Conjunto], "Set.prototype operation")?;
    let v = m.arg(a, 0);
    let oi = outro_conjunto(m, v)?;
    let eu = chaves_vivas(m, idx);
    let outro = chaves_vivas(m, oi);
    Ok(match op {
        Op::Uniao => {
            let mut r = eu;
            r.extend(outro);
            criar_conjunto(m, this, &r)
        }
        Op::Intersecao => {
            let r = filtrar(m, &eu, oi, true);
            criar_conjunto(m, this, &r)
        }
        Op::Diferenca => {
            let r = filtrar(m, &eu, oi, false);
            criar_conjunto(m, this, &r)
        }
        Op::DiferencaSimetrica => {
            let mut r = filtrar(m, &eu, oi, false);
            r.extend(filtrar(m, &outro, idx, false));
            criar_conjunto(m, this, &r)
        }
        Op::ESubconjunto => Valor::bool(filtrar(m, &eu, oi, false).is_empty()),
        Op::ESuperconjunto => Valor::bool(filtrar(m, &outro, idx, false).is_empty()),
        Op::ESdisjunto => Valor::bool(filtrar(m, &eu, oi, true).is_empty()),
    })
}

macro_rules! operacao_conjuntos {
    ($nome:ident, $op:expr) => {
        fn $nome(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
            conjuntos_op(m, this, a, $op)
        }
    };
}

operacao_conjuntos!(set_union, Op::Uniao);
operacao_conjuntos!(set_intersection, Op::Intersecao);
operacao_conjuntos!(set_difference, Op::Diferenca);
operacao_conjuntos!(set_sym_difference, Op::DiferencaSimetrica);
operacao_conjuntos!(set_is_subset_of, Op::ESubconjunto);
operacao_conjuntos!(set_is_superset_of, Op::ESuperconjunto);
operacao_conjuntos!(set_is_disjoint_from, Op::ESdisjunto);

// ---------------------------------------------------------------- WeakMap e WeakSet

fn wm_construtor(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    exigir_new(m, nt, "WeakMap")?;
    let p = proto_de_nt(m, nt)?;
    let o = novo_com(m, p, Classe::MapaFraco);
    let it = m.arg(a, 0);
    if !it.e_nulo_ou_indefinido() {
        adicionar_de(m, o, it, "set", wm_set, true, Some("Invalid value used as weak map key"))?;
    }
    Ok(o)
}

fn wm_get(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let idx = tabela_de(m, this, &[Classe::MapaFraco], "WeakMap.prototype.get")?;
    let k = m.arg(a, 0);
    Ok(ler(m, idx, k).map_or(Valor::UNDEFINED, |(_, v)| v))
}

fn wm_set(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let idx = tabela_de(m, this, &[Classe::MapaFraco], "WeakMap.prototype.set")?;
    let (k, v) = (m.arg(a, 0), m.arg(a, 1));
    validar_fraca(m, k, "Invalid value used as weak map key")?;
    inserir(m, idx, k, v);
    Ok(this)
}

fn wm_has(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let idx = tabela_de(m, this, &[Classe::MapaFraco], "WeakMap.prototype.has")?;
    let k = m.arg(a, 0);
    Ok(Valor::bool(contem(m, idx, k)))
}

fn wm_delete(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let idx = tabela_de(m, this, &[Classe::MapaFraco], "WeakMap.prototype.delete")?;
    let k = m.arg(a, 0);
    Ok(Valor::bool(apagar(m, idx, k)))
}

fn ws_construtor(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    exigir_new(m, nt, "WeakSet")?;
    let p = proto_de_nt(m, nt)?;
    let o = novo_com(m, p, Classe::ConjuntoFraco);
    let it = m.arg(a, 0);
    if !it.e_nulo_ou_indefinido() {
        adicionar_de(m, o, it, "add", ws_add, false, Some("Invalid value used in weak set"))?;
    }
    Ok(o)
}

fn ws_add(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let idx = tabela_de(m, this, &[Classe::ConjuntoFraco], "WeakSet.prototype.add")?;
    let x = m.arg(a, 0);
    validar_fraca(m, x, "Invalid value used in weak set")?;
    inserir(m, idx, x, x);
    Ok(this)
}

fn ws_has(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let idx = tabela_de(m, this, &[Classe::ConjuntoFraco], "WeakSet.prototype.has")?;
    let x = m.arg(a, 0);
    Ok(Valor::bool(contem(m, idx, x)))
}

fn ws_delete(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let idx = tabela_de(m, this, &[Classe::ConjuntoFraco], "WeakSet.prototype.delete")?;
    let x = m.arg(a, 0);
    Ok(Valor::bool(apagar(m, idx, x)))
}

// ---------------------------------------------------------------- WeakRef e FinalizationRegistry

/// `new WeakRef(alvo)`: o símbolo escondido chega como valor capturado do construtor.
fn weakref_construtor(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    exigir_new(m, nt, "WeakRef")?;
    let alvo = m.arg(a, 0);
    if !e_fraco(m, alvo) {
        return m.erro_tipo("WeakRef: target must be an object");
    }
    let segredo = m.capturado();
    let p = proto_de_nt(m, nt)?;
    let f = m.forma_para(Classe::Comum, p, 0);
    let o = m.novo_objeto(f);
    let k = m.chave_do_simbolo(segredo);
    m.definir_propria(o, k, alvo, 0);
    Ok(o)
}

fn weakref_deref(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let segredo = m.capturado();
    let k = m.chave_do_simbolo(segredo);
    if !m.e_objeto(this) || m.propria(this, k).is_none() {
        return m.erro_tipo("Method WeakRef.prototype.deref called on incompatible receiver");
    }
    m.obter(this, ChaveProp::Nome(k))
}

fn fr_construtor(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    exigir_new(m, nt, "FinalizationRegistry")?;
    let cb = m.arg(a, 0);
    if !m.e_funcao(cb) {
        return m.erro_tipo("FinalizationRegistry: cleanup must be callable");
    }
    let p = proto_de_nt(m, nt)?;
    let f = m.forma_para(Classe::Comum, p, 0);
    Ok(m.novo_objeto(f))
}

fn fr_register(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let alvo = m.arg(a, 0);
    if !e_fraco(m, alvo) {
        return m.erro_tipo("FinalizationRegistry.prototype.register: invalid target");
    }
    // shortcut: o alvo não é guardado, então o callback nunca roda
    Ok(Valor::UNDEFINED)
}

fn fr_unregister(_m: &mut Motor, _this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    // shortcut: nada foi guardado, então nada é removido
    Ok(Valor::FALSE)
}
