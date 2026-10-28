//! Object, Function, Symbol, Reflect e Proxy.

use crate::formas::{attr, Chave, Classe};
use crate::funcoes::{Args, Res, BIT_NATIVA, F_CODIGO};
use crate::intrinsecos::{simb, TipoErro};
use crate::motor::Motor;
use crate::nomes::fixos;
use crate::propriedades::{DescParcial, DescProp};
use crate::textos::ChaveProp;
use crate::valor::Valor;

pub(super) fn instalar(m: &mut Motor) {
    let g = m.intr.global;
    let op = m.intr.objeto_proto;

    // ---- Object ----
    let obj = m.nativa("Object", 1, objeto_construtor, true);
    m.definir_propria(obj, fixos::PROTOTYPE, op, 0);
    m.definir_propria(op, fixos::CONSTRUCTOR, obj, attr::METODO);
    let c = m.chave("Object");
    m.definir_propria(g, c, obj, attr::METODO);
    for (n, k, f) in [
        ("assign", 2, assign as crate::funcoes::FnNativa),
        ("create", 2, create),
        ("defineProperty", 3, define_property),
        ("defineProperties", 2, define_properties),
        ("entries", 1, entries),
        ("freeze", 1, freeze),
        ("fromEntries", 1, from_entries),
        ("getOwnPropertyDescriptor", 2, get_own_property_descriptor),
        ("getOwnPropertyDescriptors", 1, get_own_property_descriptors),
        ("getOwnPropertyNames", 1, get_own_property_names),
        ("getOwnPropertySymbols", 1, get_own_property_symbols),
        ("getPrototypeOf", 1, get_prototype_of),
        ("setPrototypeOf", 2, set_prototype_of),
        ("is", 2, is),
        ("isExtensible", 1, is_extensible),
        ("isFrozen", 1, is_frozen),
        ("isSealed", 1, is_sealed),
        ("keys", 1, keys),
        ("values", 1, values),
        ("preventExtensions", 1, prevent_extensions),
        ("seal", 1, seal),
        ("hasOwn", 2, has_own),
        ("groupBy", 2, group_by),
    ] {
        m.metodo(obj, n, k, f);
    }
    m.metodo(op, "hasOwnProperty", 1, has_own_property);
    m.metodo(op, "isPrototypeOf", 1, is_prototype_of);
    m.metodo(op, "propertyIsEnumerable", 1, property_is_enumerable);
    m.metodo(op, "toLocaleString", 0, to_locale_string);
    m.metodo(op, "valueOf", 0, value_of);
    m.metodo(op, "__defineGetter__", 2, define_getter);
    m.metodo(op, "__defineSetter__", 2, define_setter);
    m.metodo(op, "__lookupGetter__", 1, lookup_getter);
    m.metodo(op, "__lookupSetter__", 1, lookup_setter);
    let pg = m.nativa("get __proto__", 0, proto_get, false);
    let ps = m.nativa("set __proto__", 1, proto_set, false);
    m.definir_acessor(op, ChaveProp::Nome(fixos::PROTO), pg, false, false);
    m.definir_acessor(op, ChaveProp::Nome(fixos::PROTO), ps, true, false);

    // ---- Function ----
    let fp = m.intr.funcao_proto;
    let fun = m.nativa("Function", 1, funcao_construtor, true);
    m.definir_propria(fun, fixos::PROTOTYPE, fp, 0);
    m.definir_propria(fp, fixos::CONSTRUCTOR, fun, attr::METODO);
    let c = m.chave("Function");
    m.definir_propria(g, c, fun, attr::METODO);
    m.metodo(fp, "call", 1, call);
    m.metodo(fp, "apply", 2, apply);
    m.metodo(fp, "bind", 1, bind);
    m.metodo(fp, "toString", 0, funcao_to_string);

    // ---- Symbol ----
    let sp = m.intr.simbolo_proto;
    let sym = m.nativa("Symbol", 0, simbolo_construtor, false);
    m.definir_propria(sym, fixos::PROTOTYPE, sp, 0);
    m.definir_propria(sp, fixos::CONSTRUCTOR, sym, attr::METODO);
    let c = m.chave("Symbol");
    m.definir_propria(g, c, sym, attr::METODO);
    let nomes = ["iterator", "asyncIterator", "hasInstance", "toPrimitive", "toStringTag", "species", "isConcatSpreadable", "unscopables", "match", "matchAll", "replace", "search", "split"];
    for (i, n) in nomes.iter().enumerate() {
        let s = m.simbolos[i];
        let c = m.chave(n);
        m.definir_propria(sym, c, s, 0);
    }
    m.escondidos.insert(simb::CAMPOS.0 & !Chave::BIT_SIMBOLO);
    m.metodo(sym, "for", 1, simbolo_for);
    m.metodo(sym, "keyFor", 1, simbolo_key_for);
    m.metodo(sp, "toString", 0, simbolo_to_string);
    m.metodo(sp, "valueOf", 0, simbolo_value_of);
    let dg = m.nativa("get description", 0, simbolo_descricao, false);
    let c = m.chave("description");
    m.definir_acessor(sp, ChaveProp::Nome(c), dg, false, false);
    let tag = m.texto("Symbol");
    m.definir_propria(sp, simb::TO_STRING_TAG, tag, attr::CONFIGURAVEL);
    let tp = m.nativa("[Symbol.toPrimitive]", 1, simbolo_value_of, false);
    m.definir_propria(sp, simb::TO_PRIMITIVE, tp, attr::CONFIGURAVEL);

    // ---- Reflect ----
    let r = m.objeto_vazio();
    for (n, k, f) in [
        ("apply", 3, reflect_apply as crate::funcoes::FnNativa),
        ("construct", 2, reflect_construct),
        ("defineProperty", 3, reflect_define_property),
        ("deleteProperty", 2, reflect_delete_property),
        ("get", 2, reflect_get),
        ("getOwnPropertyDescriptor", 2, get_own_property_descriptor),
        ("getPrototypeOf", 1, get_prototype_of),
        ("has", 2, reflect_has),
        ("isExtensible", 1, is_extensible),
        ("ownKeys", 1, reflect_own_keys),
        ("preventExtensions", 1, reflect_prevent_extensions),
        ("set", 3, reflect_set),
        ("setPrototypeOf", 2, reflect_set_prototype_of),
    ] {
        m.metodo(r, n, k, f);
    }
    let tag = m.texto("Reflect");
    m.definir_propria(r, simb::TO_STRING_TAG, tag, attr::CONFIGURAVEL);
    let c = m.chave("Reflect");
    m.definir_propria(g, c, r, attr::METODO);

    // ---- Proxy ----
    let px = m.nativa("Proxy", 2, proxy_construtor, true);
    m.metodo(px, "revocable", 2, proxy_revocable);
    let c = m.chave("Proxy");
    m.definir_propria(g, c, px, attr::METODO);

    m.definir_propria(g, fixos::GLOBAL_THIS, g, attr::METODO);
}

// ---------------------------------------------------------------- auxiliares

fn exigir_objeto(m: &mut Motor, v: Valor, onde: &str) -> Res<Valor> {
    if m.e_objeto(v) {
        Ok(v)
    } else {
        m.erro_tipo(&format!("{onde} called on non-object"))
    }
}

/// Descritor em objeto JS (`{value, writable, …}`).
pub(crate) fn descritor_para_objeto(m: &mut Motor, d: DescProp) -> Valor {
    let o = m.objeto_vazio();
    if d.acessor() {
        m.definir_propria(o, fixos::GET, d.get, attr::PADRAO);
        m.definir_propria(o, fixos::SET, d.set, attr::PADRAO);
    } else {
        m.definir_propria(o, fixos::VALUE, d.valor, attr::PADRAO);
        m.definir_propria(o, fixos::WRITABLE, Valor::bool(d.attrs & attr::GRAVAVEL != 0), attr::PADRAO);
    }
    m.definir_propria(o, fixos::ENUMERABLE, Valor::bool(d.attrs & attr::ENUMERAVEL != 0), attr::PADRAO);
    m.definir_propria(o, fixos::CONFIGURABLE, Valor::bool(d.attrs & attr::CONFIGURAVEL != 0), attr::PADRAO);
    o
}

/// ToPropertyDescriptor.
pub(crate) fn objeto_para_descritor(m: &mut Motor, o: Valor) -> Res<DescParcial> {
    if !m.e_objeto(o) {
        let d = m.descrever(o);
        return m.erro_tipo(&format!("Property description must be an object: {d}"));
    }
    let mut d = DescParcial::default();
    let ler = |m: &mut Motor, c: Chave| -> Res<Option<Valor>> {
        if m.tem(o, ChaveProp::Nome(c))? {
            Ok(Some(m.obter(o, ChaveProp::Nome(c))?))
        } else {
            Ok(None)
        }
    };
    if let Some(v) = ler(m, fixos::ENUMERABLE)? {
        d.enumeravel = Some(m.verdadeiro(v));
    }
    if let Some(v) = ler(m, fixos::CONFIGURABLE)? {
        d.configuravel = Some(m.verdadeiro(v));
    }
    d.valor = ler(m, fixos::VALUE)?;
    if let Some(v) = ler(m, fixos::WRITABLE)? {
        d.gravavel = Some(m.verdadeiro(v));
    }
    for (c, alvo) in [(fixos::GET, &mut d.get), (fixos::SET, &mut d.set)] {
        if let Some(f) = ler(m, c)? {
            if f != Valor::UNDEFINED && !m.e_funcao(f) {
                return m.erro_tipo("Getter/setter must be a function");
            }
            *alvo = Some(f);
        }
    }
    if (d.get.is_some() || d.set.is_some()) && (d.valor.is_some() || d.gravavel.is_some()) {
        return m.erro_tipo("Invalid property descriptor. Cannot both specify accessors and a value or writable attribute");
    }
    Ok(d)
}

impl Motor {
    /// [`objeto_para_descritor`] para quem está fora de `builtins` (armadilhas de Proxy).
    pub(crate) fn para_descritor(&mut self, o: Valor) -> Res<DescParcial> {
        objeto_para_descritor(self, o)
    }
}

fn chaves_para_array(m: &mut Motor, ks: Vec<ChaveProp>) -> Valor {
    let vs: Vec<Valor> = ks.into_iter().map(|k| m.chave_para_valor(k)).collect();
    m.array_de(&vs)
}

/// Torna todas as próprias não configuráveis (e não graváveis, se `congelar`) e fecha.
fn integridade(m: &mut Motor, o: Valor, congelar: bool) -> Res<()> {
    if m.formas.get(m.forma(o)).classe == Classe::Array && m.array_len(o) > 0 {
        // Itens indexados de array ficam como nomes com atributos (raro: só em freeze/seal).
        for k in m.chaves_de(o, false, true, false) {
            if let ChaveProp::Indice(_) = k {
                let mut d = DescParcial { configuravel: Some(false), ..Default::default() };
                if congelar {
                    d.gravavel = Some(false);
                }
                m.definir_propriedade(o, k, d)?;
            }
        }
    }
    if congelar {
        if m.e_array(o) {
            let n = m.array_len(o);
            m.definir_propria(o, fixos::LENGTH, Valor::int(n as i32), 0);
        }
        m.congelar(o);
    } else {
        for d in m.chaves_proprias(o) {
            let a = d.attrs & !attr::CONFIGURAVEL;
            if a != d.attrs {
                let nf = m.formas.mudar_atributos(m.forma(o), d.chave, a);
                m.g.por_u32(o.deslocamento(), nf.0);
            }
        }
        let nf = m.formas.fechar(m.forma(o));
        m.g.por_u32(o.deslocamento(), nf.0);
    }
    Ok(())
}

fn testar_integridade(m: &mut Motor, o: Valor, congelado: bool) -> bool {
    if !m.e_objeto(o) {
        return true;
    }
    if m.formas.get(m.forma(o)).extensivel {
        return false;
    }
    for k in m.chaves_de(o, false, true, true) {
        if let Some(d) = m.descritor_proprio(o, k) {
            if d.attrs & attr::CONFIGURAVEL != 0 {
                return false;
            }
            if congelado && !d.acessor() && d.attrs & attr::GRAVAVEL != 0 {
                return false;
            }
        }
    }
    true
}

// ---------------------------------------------------------------- Object

fn objeto_construtor(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    if nt != Valor::UNDEFINED && nt != m.chamado {
        let op = m.intr.objeto_proto;
        return m.objeto_de_construtor(nt, op);
    }
    if v.e_nulo_ou_indefinido() {
        return Ok(m.objeto_vazio());
    }
    m.para_objeto(v)
}

fn assign(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let alvo = m.para_objeto(m.arg(a, 0))?;
    for i in 1..a.n {
        let src = m.arg(a, i);
        if src.e_nulo_ou_indefinido() {
            continue;
        }
        let src = m.para_objeto(src)?;
        for k in m.chaves_de(src, true, true, true) {
            let v = m.obter(src, k)?;
            m.gravar(alvo, k, v, true)?;
        }
    }
    Ok(alvo)
}

fn create(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let p = m.arg(a, 0);
    if !m.e_objeto(p) && p != Valor::NULL {
        return m.erro_tipo("Object prototype may only be an Object or null");
    }
    let f = m.forma_para(Classe::Comum, p, 4);
    let o = m.novo_objeto(f);
    let props = m.arg(a, 1);
    if props != Valor::UNDEFINED {
        definir_varias(m, o, props)?;
    }
    Ok(o)
}

fn definir_varias(m: &mut Motor, o: Valor, props: Valor) -> Res<()> {
    let props = m.para_objeto(props)?;
    let mut lista = Vec::new();
    for k in m.chaves_de(props, true, true, true) {
        let dv = m.obter(props, k)?;
        lista.push((k, objeto_para_descritor(m, dv)?));
    }
    for (k, d) in lista {
        if !m.definir_propriedade(o, k, d)? {
            let n = m.chave_para_rust(k);
            return m.erro_tipo(&format!("Cannot redefine property: {n}"));
        }
    }
    Ok(())
}

fn define_property(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.arg(a, 0);
    exigir_objeto(m, o, "Object.defineProperty")?;
    let k = m.para_chave(m.arg(a, 1))?;
    let d = objeto_para_descritor(m, m.arg(a, 2))?;
    if !m.definir_propriedade(o, k, d)? {
        let n = m.chave_para_rust(k);
        return m.erro_tipo(&format!("Cannot redefine property: {n}"));
    }
    Ok(o)
}

fn define_properties(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.arg(a, 0);
    exigir_objeto(m, o, "Object.defineProperties")?;
    definir_varias(m, o, m.arg(a, 1))?;
    Ok(o)
}

fn entradas(m: &mut Motor, v: Valor, tipo: u8) -> Res<Valor> {
    let o = m.para_objeto(v)?;
    let mut r = Vec::new();
    for k in m.chaves_de(o, true, true, false) {
        let ok = m.descritor_proprio(o, k).is_some_and(|d| d.attrs & attr::ENUMERAVEL != 0);
        if !ok {
            continue;
        }
        let kv = m.chave_para_valor(k);
        r.push(match tipo {
            0 => kv,
            1 => m.obter(o, k)?,
            _ => {
                let v = m.obter(o, k)?;
                m.array_de(&[kv, v])
            }
        });
    }
    Ok(m.array_de(&r))
}

fn keys(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    entradas(m, m.arg(a, 0), 0)
}

fn values(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    entradas(m, m.arg(a, 0), 1)
}

fn entries(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    entradas(m, m.arg(a, 0), 2)
}

fn freeze(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.arg(a, 0);
    if m.e_objeto(o) {
        integridade(m, o, true)?;
    }
    Ok(o)
}

fn seal(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.arg(a, 0);
    if m.e_objeto(o) {
        integridade(m, o, false)?;
    }
    Ok(o)
}

fn prevent_extensions(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.arg(a, 0);
    if m.e_objeto(o) {
        let nf = m.formas.fechar(m.forma(o));
        m.g.por_u32(o.deslocamento(), nf.0);
    }
    Ok(o)
}

fn is_frozen(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    Ok(Valor::bool(testar_integridade(m, m.arg(a, 0), true)))
}

fn is_sealed(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    Ok(Valor::bool(testar_integridade(m, m.arg(a, 0), false)))
}

fn is_extensible(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.arg(a, 0);
    Ok(Valor::bool(m.e_objeto(o) && m.formas.get(m.forma(o)).extensivel))
}

fn from_entries(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.objeto_vazio();
    m.percorrer_iteravel(m.arg(a, 0), |m, par, _| {
        if !m.e_objeto(par) {
            return m.erro_tipo("Iterator value is not an entry object");
        }
        let k = m.obter(par, ChaveProp::Indice(0))?;
        let v = m.obter(par, ChaveProp::Indice(1))?;
        let k = m.para_chave(k)?;
        m.criar_dado(o, k, v)?;
        Ok(())
    })?;
    Ok(o)
}

fn get_own_property_descriptor(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(m.arg(a, 0))?;
    let k = m.para_chave(m.arg(a, 1))?;
    Ok(match m.descritor_proprio_res(o, k)? {
        Some(d) => descritor_para_objeto(m, d),
        None => Valor::UNDEFINED,
    })
}

fn get_own_property_descriptors(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(m.arg(a, 0))?;
    let r = m.objeto_vazio();
    for k in m.chaves_de(o, false, true, true) {
        if let Some(d) = m.descritor_proprio(o, k) {
            let dv = descritor_para_objeto(m, d);
            m.criar_dado(r, k, dv)?;
        }
    }
    Ok(r)
}

fn get_own_property_names(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(m.arg(a, 0))?;
    let ks = m.chaves_de(o, false, true, false);
    Ok(chaves_para_array(m, ks))
}

fn get_own_property_symbols(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(m.arg(a, 0))?;
    let ks = m.chaves_de(o, false, false, true);
    Ok(chaves_para_array(m, ks))
}

fn get_prototype_of(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(m.arg(a, 0))?;
    Ok(m.proto_de(o))
}

fn set_prototype_of(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (o, p) = (m.arg(a, 0), m.arg(a, 1));
    if o.e_nulo_ou_indefinido() {
        return m.erro_tipo("Object.setPrototypeOf called on null or undefined");
    }
    if !m.e_objeto(p) && p != Valor::NULL {
        return m.erro_tipo("Object prototype may only be an Object or null");
    }
    if m.e_objeto(o) && !mudar_proto(m, o, p) {
        return m.erro_tipo("Cyclic __proto__ value or object not extensible");
    }
    Ok(o)
}

/// Troca o protótipo conferindo ciclo e extensibilidade.
fn mudar_proto(m: &mut Motor, o: Valor, p: Valor) -> bool {
    if m.proto_de(o) == p {
        return true;
    }
    if !m.formas.get(m.forma(o)).extensivel {
        return false;
    }
    let mut x = p;
    while x != Valor::NULL {
        if x == o {
            return false;
        }
        x = m.proto_de(x);
    }
    m.definir_proto(o, p);
    true
}

fn is(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (x, y) = (m.arg(a, 0), m.arg(a, 1));
    Ok(Valor::bool(m.mesmo_valor(x, y)))
}

fn has_own(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(m.arg(a, 0))?;
    let k = m.para_chave(m.arg(a, 1))?;
    Ok(Valor::bool(m.tem_propria(o, k)))
}

fn group_by(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (itens, f) = (m.arg(a, 0), m.arg(a, 1));
    if itens.e_nulo_ou_indefinido() {
        return m.erro_tipo("Object.groupBy called on null or undefined");
    }
    if !m.e_funcao(f) {
        let d = m.descrever(f);
        return m.erro_tipo(&format!("{d} is not a function"));
    }
    let r = m.forma_para(Classe::Comum, Valor::NULL, 4);
    let r = m.novo_objeto(r);
    m.percorrer_iteravel(itens, |m, v, i| {
        let iv = m.numero(i as f64);
        let k = m.chamar(f, Valor::UNDEFINED, &[v, iv])?;
        let k = m.para_chave(k)?;
        let mut grupo = m.obter(r, k)?;
        if grupo == Valor::UNDEFINED {
            grupo = m.array_de(&[]);
            m.criar_dado(r, k, grupo)?;
        }
        let n = m.array_len(grupo);
        m.gravar_elemento(grupo, n, v);
        Ok(())
    })?;
    Ok(r)
}

fn has_own_property(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let k = m.para_chave(m.arg(a, 0))?;
    let o = m.para_objeto(this)?;
    Ok(Valor::bool(m.tem_propria(o, k)))
}

fn is_prototype_of(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    if !m.e_objeto(v) {
        return Ok(Valor::FALSE);
    }
    let o = m.para_objeto(this)?;
    let mut p = m.proto_de(v);
    while p != Valor::NULL {
        if p == o {
            return Ok(Valor::TRUE);
        }
        p = m.proto_de(p);
    }
    Ok(Valor::FALSE)
}

fn property_is_enumerable(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let k = m.para_chave(m.arg(a, 0))?;
    let o = m.para_objeto(this)?;
    Ok(Valor::bool(m.descritor_proprio(o, k).is_some_and(|d| d.attrs & attr::ENUMERAVEL != 0)))
}

fn to_locale_string(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let f = m.obter(this, ChaveProp::Nome(fixos::TO_STRING))?;
    m.chamar_coletavel(f, this, &[])
}

fn value_of(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    m.para_objeto(this)
}

fn define_getter(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let k = m.para_chave(m.arg(a, 0))?;
    let f = m.arg(a, 1);
    if !m.e_funcao(f) {
        return m.erro_tipo("Object.prototype.__defineGetter__: Expecting function");
    }
    m.definir_acessor(o, k, f, false, true);
    Ok(Valor::UNDEFINED)
}

fn define_setter(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let k = m.para_chave(m.arg(a, 0))?;
    let f = m.arg(a, 1);
    if !m.e_funcao(f) {
        return m.erro_tipo("Object.prototype.__defineSetter__: Expecting function");
    }
    m.definir_acessor(o, k, f, true, true);
    Ok(Valor::UNDEFINED)
}

fn lookup_getter(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    lookup_acessor(m, this, a, false)
}

fn lookup_setter(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    lookup_acessor(m, this, a, true)
}

/// `__lookupGetter__` (`setter` falso) e `__lookupSetter__` (verdadeiro), Anexo B.
fn lookup_acessor(m: &mut Motor, this: Valor, a: Args, setter: bool) -> Res<Valor> {
    let mut o = m.para_objeto(this)?;
    let k = m.para_chave(m.arg(a, 0))?;
    while o != Valor::NULL {
        if let Some(d) = m.descritor_proprio(o, k) {
            return Ok(if !d.acessor() {
                Valor::UNDEFINED
            } else if setter {
                d.set
            } else {
                d.get
            });
        }
        o = m.proto_de(o);
    }
    Ok(Valor::UNDEFINED)
}

fn proto_get(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    Ok(m.proto_de(o))
}

fn proto_set(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let p = m.arg(a, 0);
    if m.e_objeto(this) && (m.e_objeto(p) || p == Valor::NULL) && !mudar_proto(m, this, p) {
        return m.erro_tipo("Cyclic __proto__ value");
    }
    Ok(Valor::UNDEFINED)
}

// ---------------------------------------------------------------- Function

fn funcao_construtor(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let mut params = Vec::new();
    for i in 0..a.n.saturating_sub(1) {
        params.push(m.para_texto_rust(m.arg(a, i))?);
    }
    let corpo = if a.n > 0 { m.para_texto_rust(m.arg(a, a.n - 1))? } else { String::new() };
    let fonte = format!("(function anonymous({}\n) {{\n{}\n}})", params.join(","), corpo);
    let p = m.compilar_texto(&fonte)?;
    m.executar(p)
}

fn call(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    if !m.e_funcao(this) {
        return m.erro_tipo("Function.prototype.call called on non-function");
    }
    let t = m.arg(a, 0);
    let resto: Vec<Valor> = (1..a.n.max(1)).map(|i| m.arg(a, i)).collect();
    m.chamar_coletavel(this, t, &resto)
}

/// Argumentos numa chamada por lista (`apply`, `Reflect.apply/construct`). shortcut: teto fixo,
/// perto do do SpiderMonkey (500 mil); sobe se aparecer uso legítimo maior.
pub(crate) const LIMITE_ARGS: u64 = 1 << 20;

/// CreateListFromArrayLike.
pub(crate) fn lista_de_array(m: &mut Motor, v: Valor) -> Res<Vec<Valor>> {
    if v.e_nulo_ou_indefinido() {
        return Ok(Vec::new());
    }
    if !m.e_objeto(v) {
        return m.erro_tipo("CreateListFromArrayLike called on non-object");
    }
    let n = m.comprimento_de(v)?;
    // Uma lista de argumentos sem teto (`f.apply(null, {length: 2**32})`) leva gigabytes.
    if n > LIMITE_ARGS {
        return m.erro_alcance("Too many arguments in function call");
    }
    let mut r = Vec::with_capacity(n as usize);
    for i in 0..n {
        r.push(m.obter_indice(v, i)?);
    }
    Ok(r)
}

fn apply(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    if !m.e_funcao(this) {
        return m.erro_tipo("Function.prototype.apply was called on a non-function");
    }
    let t = m.arg(a, 0);
    let args = lista_de_array(m, m.arg(a, 1))?;
    m.chamar_coletavel(this, t, &args)
}

fn bind(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    if !m.e_funcao(this) {
        return m.erro_tipo("Bind must be called on a function");
    }
    let t = m.arg(a, 0);
    let n = a.n.saturating_sub(1);
    let lista = m.nova_lista(n);
    for i in 0..n {
        let v = m.arg(a, i + 1);
        m.lista_set(lista, i, v);
    }
    let p = m.proto_de(this);
    let f = m.forma_para(Classe::FuncaoLigada, p, 2);
    let b = m.novo_objeto(f);
    m.por_interno_valor(b, 0, this);
    m.por_interno_valor(b, 1, t);
    m.por_interno_valor(b, 2, lista);
    let len = if m.tem_propria(this, ChaveProp::Nome(fixos::LENGTH)) {
        let l = m.obter(this, ChaveProp::Nome(fixos::LENGTH))?;
        if m.e_numero(l) {
            (m.f64_de(l) - f64::from(n)).max(0.0)
        } else {
            0.0
        }
    } else {
        0.0
    };
    let lv = m.numero(len);
    m.definir_propria(b, fixos::LENGTH, lv, attr::CONFIGURAVEL);
    let nome = m.obter(this, ChaveProp::Nome(fixos::NAME))?;
    let nome = if m.e_texto(nome) { nome } else { Valor::VAZIA };
    let pre = m.texto("bound ");
    let nome = m.concatenar(pre, nome)?;
    m.definir_propria(b, fixos::NAME, nome, attr::CONFIGURAVEL);
    Ok(b)
}

fn funcao_to_string(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    if !m.e_funcao(this) {
        return m.erro_tipo("Function.prototype.toString requires that 'this' be a Function");
    }
    if m.formas.get(m.forma(this)).classe == Classe::Funcao {
        let c = m.interno(this, F_CODIGO);
        if c & BIT_NATIVA == 0 {
            let p = &m.protos[c as usize];
            let fb = p.codigo();
            if let Some(f) = &p.fonte {
                let (i, j) = (fb.inicio as usize, fb.fim as usize);
                if let Some(s) = f.get(i..j) {
                    let s = s.to_string();
                    return Ok(m.texto(&s));
                }
            }
        }
    }
    let nome = m.obter(this, ChaveProp::Nome(fixos::NAME))?;
    let nome = if m.e_texto(nome) { m.texto_rust(nome) } else { String::new() };
    Ok(m.texto(&format!("function {nome}() {{ [native code] }}")))
}

// ---------------------------------------------------------------- Symbol

fn simbolo_construtor(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    if nt != Valor::UNDEFINED {
        return m.erro_tipo("Symbol is not a constructor");
    }
    let d = m.arg(a, 0);
    let d = if d == Valor::UNDEFINED { d } else { m.para_texto(d)? };
    Ok(m.novo_simbolo(d))
}

fn simbolo_for(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = m.para_texto(m.arg(a, 0))?;
    let u = m.texto_u16_de(t);
    if let Some(&s) = m.registro_simbolos.get(&u) {
        return Ok(s);
    }
    let s = m.novo_simbolo(t);
    m.registro_simbolos.insert(u, s);
    Ok(s)
}

fn simbolo_key_for(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let s = m.arg(a, 0);
    if !m.e_simbolo(s) {
        return m.erro_tipo("Symbol.keyFor: argument is not a symbol");
    }
    if m.registro_simbolos.values().any(|&x| x == s) {
        return Ok(m.descricao_do_simbolo(s));
    }
    Ok(Valor::UNDEFINED)
}

fn este_simbolo(m: &mut Motor, this: Valor) -> Res<Valor> {
    if m.e_simbolo(this) {
        return Ok(this);
    }
    if m.e_objeto(this) && m.formas.get(m.forma(this)).classe == Classe::Caixa {
        let p = Valor(m.interno(this, 0));
        if m.e_simbolo(p) {
            return Ok(p);
        }
    }
    m.erro_tipo("Symbol.prototype method called on incompatible receiver")
}

fn simbolo_to_string(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let s = este_simbolo(m, this)?;
    let k = m.chave_do_simbolo(s);
    let t = m.chave_para_rust(ChaveProp::Nome(k));
    Ok(m.texto(&t))
}

fn simbolo_value_of(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    este_simbolo(m, this)
}

fn simbolo_descricao(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let s = este_simbolo(m, this)?;
    Ok(m.descricao_do_simbolo(s))
}

// ---------------------------------------------------------------- Reflect

fn reflect_apply(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (f, t) = (m.arg(a, 0), m.arg(a, 1));
    if !m.e_funcao(f) {
        return m.erro_tipo("Reflect.apply target is not a function");
    }
    let args = lista_de_array(m, m.arg(a, 2))?;
    m.chamar_coletavel(f, t, &args)
}

fn reflect_construct(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let f = m.arg(a, 0);
    let nt = if a.n > 2 { m.arg(a, 2) } else { f };
    if !m.e_construtor(f) || !m.e_construtor(nt) {
        return m.erro_tipo("Reflect.construct target is not a constructor");
    }
    let args = lista_de_array(m, m.arg(a, 1))?;
    m.construir(f, &args, nt)
}

fn reflect_define_property(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.arg(a, 0);
    exigir_objeto(m, o, "Reflect.defineProperty")?;
    let k = m.para_chave(m.arg(a, 1))?;
    let d = objeto_para_descritor(m, m.arg(a, 2))?;
    Ok(Valor::bool(m.definir_propriedade(o, k, d)?))
}

fn reflect_delete_property(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.arg(a, 0);
    exigir_objeto(m, o, "Reflect.deleteProperty")?;
    let k = m.para_chave(m.arg(a, 1))?;
    Ok(Valor::bool(m.apagar(o, k)?))
}

fn reflect_get(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.arg(a, 0);
    exigir_objeto(m, o, "Reflect.get")?;
    let k = m.para_chave(m.arg(a, 1))?;
    let r = if a.n > 2 { m.arg(a, 2) } else { o };
    m.obter_com(o, k, r)
}

fn reflect_has(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.arg(a, 0);
    exigir_objeto(m, o, "Reflect.has")?;
    let k = m.para_chave(m.arg(a, 1))?;
    Ok(Valor::bool(m.tem(o, k)?))
}

fn reflect_own_keys(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.arg(a, 0);
    exigir_objeto(m, o, "Reflect.ownKeys")?;
    let ks = m.chaves_proprias_res(o)?;
    Ok(chaves_para_array(m, ks))
}

fn reflect_prevent_extensions(m: &mut Motor, this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    let o = m.arg(a, 0);
    exigir_objeto(m, o, "Reflect.preventExtensions")?;
    prevent_extensions(m, this, a, nt)?;
    Ok(Valor::TRUE)
}

fn reflect_set(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.arg(a, 0);
    exigir_objeto(m, o, "Reflect.set")?;
    let k = m.para_chave(m.arg(a, 1))?;
    let v = m.arg(a, 2);
    let r = if a.n > 3 { m.arg(a, 3) } else { o };
    Ok(Valor::bool(m.gravar_com(o, k, v, r)?))
}

fn reflect_set_prototype_of(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (o, p) = (m.arg(a, 0), m.arg(a, 1));
    exigir_objeto(m, o, "Reflect.setPrototypeOf")?;
    if !m.e_objeto(p) && p != Valor::NULL {
        return m.erro_tipo("Object prototype may only be an Object or null");
    }
    Ok(Valor::bool(mudar_proto(m, o, p)))
}

// ---------------------------------------------------------------- Proxy

fn criar_proxy(m: &mut Motor, alvo: Valor, tratador: Valor) -> Res<Valor> {
    if !m.e_objeto(alvo) || !m.e_objeto(tratador) {
        return m.erro_tipo("Cannot create proxy with a non-object as target or handler");
    }
    // shortcut: getPrototypeOf do proxy é o do alvo na criação (sem armadilha).
    let p = m.proto_de(alvo);
    let f = m.forma_para(Classe::Proxy, p, 0);
    let o = m.novo_objeto(f);
    m.por_interno_valor(o, 0, alvo);
    m.por_interno_valor(o, 1, tratador);
    Ok(o)
}

fn proxy_construtor(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    if nt == Valor::UNDEFINED {
        return m.erro_tipo("Constructor Proxy requires 'new'");
    }
    criar_proxy(m, m.arg(a, 0), m.arg(a, 1))
}

fn revogar(m: &mut Motor, _this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let p = m.capturado();
    if m.e_objeto(p) {
        m.por_interno_valor(p, 1, Valor::NULL);
    }
    Ok(Valor::UNDEFINED)
}

fn proxy_revocable(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let p = criar_proxy(m, m.arg(a, 0), m.arg(a, 1))?;
    let id = m.id_nativa("revogar_proxy", revogar);
    let r = m.nativa_com(id, 0, p);
    let o = m.objeto_vazio();
    let kp = m.chave("proxy");
    let kr = m.chave("revoke");
    m.definir_propria(o, kp, p, attr::PADRAO);
    m.definir_propria(o, kr, r, attr::PADRAO);
    let _ = TipoErro::Tipo;
    Ok(o)
}
