//! O núcleo: construtores padrão de classe, `eval`, `console`, erros, iteradores de array,
//! geradores, `Symbol.hasInstance` e o `toString` básico.

use crate::formas::{attr, Classe};
use crate::funcoes::{Args, Res};
use crate::interpretador::{Entrada, Fim};
use crate::intrinsecos::{simb, TipoErro};
use crate::motor::Motor;
use crate::nomes::fixos;
use crate::textos::ChaveProp;
use crate::valor::Valor;

pub(super) fn instalar(m: &mut Motor) {
    m.intr.nativa_classe_base = m.registrar_nativa("", classe_base, true);
    m.intr.nativa_classe_derivada = m.registrar_nativa("", classe_derivada, true);
    let g = m.intr.global;
    m.intr.eval = m.metodo(g, "eval", 1, eval);

    // Function.prototype
    let fp = m.intr.funcao_proto;
    let hi = m.nativa("[Symbol.hasInstance]", 1, has_instance, false);
    m.definir_propria(fp, simb::HAS_INSTANCE, hi, 0);
    m.intr.funcao_has_instance = hi;

    // Object.prototype.toString mínimo (o resto vem em `objeto`).
    let op = m.intr.objeto_proto;
    m.metodo(op, "toString", 0, objeto_to_string);

    // Erros.
    let nomes = ["Error", "TypeError", "ReferenceError", "SyntaxError", "RangeError", "URIError", "EvalError", "AggregateError"];
    let tipos = [TipoErro::Erro, TipoErro::Tipo, TipoErro::Referencia, TipoErro::Sintaxe, TipoErro::Alcance, TipoErro::Uri, TipoErro::Eval, TipoErro::Agregado];
    let mut erro_ctor = Valor::UNDEFINED;
    for (i, nome) in nomes.iter().enumerate() {
        let id = m.registrar_nativa(nome, erro_construtor, true);
        let ctor = m.nativa_com(id, 1, Valor::int(tipos[i] as i32));
        let n = m.texto(nome);
        m.gravar_campo(ctor, 1, n);
        let p = m.intr.erro_protos[i];
        m.definir_propria(ctor, fixos::PROTOTYPE, p, 0);
        m.definir_propria(p, fixos::CONSTRUCTOR, ctor, attr::METODO);
        m.definir_propria(p, fixos::NAME, n, attr::METODO);
        m.definir_propria(p, fixos::MESSAGE, Valor::VAZIA, attr::METODO);
        if i == 0 {
            erro_ctor = ctor;
            m.metodo(p, "toString", 0, erro_to_string);
        } else {
            m.definir_proto(ctor, erro_ctor);
        }
        let c = m.chave(nome);
        m.definir_propria(g, c, ctor, attr::METODO);
    }

    // Iterador de array (Array.prototype.values/[Symbol.iterator]).
    let ip = m.intr.iterador_proto;
    let ii = m.nativa("[Symbol.iterator]", 0, devolve_this, false);
    m.definir_propria(ip, simb::ITERATOR, ii, attr::METODO);
    let aip = m.intr.array_iterador_proto;
    m.intr.array_iterador_next = m.metodo(aip, "next", 0, array_iterador_next);
    let ap = m.intr.array_proto;
    let vals = m.metodo(ap, "values", 0, array_values);
    m.definir_propria(ap, simb::ITERATOR, vals, attr::METODO);
    m.intr.array_valores = vals;
    m.metodo(ap, "keys", 0, array_keys);
    m.metodo(ap, "entries", 0, array_entries);

    // Geradores.
    let gp = m.intr.gerador_proto;
    m.metodo(gp, "next", 1, gerador_next);
    m.metodo(gp, "return", 1, gerador_return);
    m.metodo(gp, "throw", 1, gerador_throw);
    let gfp = m.intr.gerador_funcao_proto;
    m.definir_propria(gfp, fixos::PROTOTYPE, gp, attr::CONFIGURAVEL);

    // console
    let console = m.objeto_vazio();
    m.metodo(console, "log", 0, console_log);
    m.metodo(console, "info", 0, console_info);
    m.metodo(console, "debug", 0, console_debug);
    m.metodo(console, "warn", 0, console_warn);
    m.metodo(console, "error", 0, console_error);
    let c = m.chave("console");
    m.definir_propria(g, c, console, attr::METODO);
}

// ---------------------------------------------------------------- classes

fn classe_base(m: &mut Motor, _this: Valor, _a: Args, nt: Valor) -> Res<Valor> {
    if nt == Valor::UNDEFINED {
        return m.erro_tipo("Class constructor cannot be invoked without 'new'");
    }
    let f = m.chamado;
    let op = m.intr.objeto_proto;
    let o = m.objeto_de_construtor(nt, op)?;
    m.rodar_campos(f, o)?;
    Ok(o)
}

fn classe_derivada(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    if nt == Valor::UNDEFINED {
        return m.erro_tipo("Class constructor cannot be invoked without 'new'");
    }
    let f = m.chamado;
    let pai = m.proto_de(f);
    if !m.e_construtor(pai) {
        return m.erro_tipo("Super constructor is not a constructor");
    }
    let args: Vec<Valor> = (0..a.n).map(|i| m.arg(a, i)).collect();
    let o = m.construir(pai, &args, nt)?;
    m.rodar_campos(f, o)?;
    Ok(o)
}

fn eval(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let x = m.arg(a, 0);
    if !m.e_texto(x) {
        return Ok(x);
    }
    let s = m.texto_rust(x);
    let p = m.compilar_texto(&s)?;
    m.executar(p)
}

fn has_instance(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    Ok(Valor::bool(m.instancia_comum(this, v)?))
}

fn devolve_this(_m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    Ok(this)
}

/// Marca de classe para `Object.prototype.toString`.
pub(crate) fn marca(m: &mut Motor, v: Valor) -> Res<String> {
    if v == Valor::UNDEFINED {
        return Ok("Undefined".into());
    }
    if v == Valor::NULL {
        return Ok("Null".into());
    }
    let o = m.para_objeto(v)?;
    let base = match m.formas.get(m.forma(o)).classe {
        Classe::Array => "Array",
        Classe::Funcao | Classe::FuncaoLigada => "Function",
        Classe::Erro => "Error",
        Classe::Data => "Date",
        Classe::Regex => "RegExp",
        Classe::Argumentos => "Arguments",
        Classe::Caixa => {
            let p = Valor(m.interno(o, 0));
            if m.e_texto(p) {
                "String"
            } else if m.e_numero(p) {
                "Number"
            } else if p == Valor::TRUE || p == Valor::FALSE {
                "Boolean"
            } else {
                "Object"
            }
        }
        _ => "Object",
    };
    let tag = m.obter(o, ChaveProp::Nome(simb::TO_STRING_TAG))?;
    if m.e_texto(tag) {
        return Ok(m.texto_rust(tag));
    }
    Ok(base.into())
}

fn objeto_to_string(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let t = marca(m, this)?;
    Ok(m.texto(&format!("[object {t}]")))
}

// ---------------------------------------------------------------- erros

fn erro_construtor(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    let tipo = m.capturado().como_int() as usize;
    let f = m.chamado;
    let nt = if nt == Valor::UNDEFINED { f } else { nt };
    let padrao = m.intr.erro_protos[tipo];
    let p = m.obter(nt, ChaveProp::Nome(fixos::PROTOTYPE))?;
    let p = if m.e_objeto(p) { p } else { padrao };
    let forma = m.forma_para(Classe::Erro, p, 2);
    let e = m.novo_objeto(forma);
    let (msg_i, opc_i) = if tipo == TipoErro::Agregado as usize { (1, 2) } else { (0, 1) };
    if tipo == TipoErro::Agregado as usize {
        let itens = m.coletar_iteravel(m.arg(a, 0))?;
        let arr = m.array_de(&itens);
        m.definir_propria(e, fixos::ERRORS, arr, attr::METODO);
    }
    let msg = m.arg(a, msg_i);
    if msg != Valor::UNDEFINED {
        let t = m.para_texto(msg)?;
        m.definir_propria(e, fixos::MESSAGE, t, attr::METODO);
    }
    let opc = m.arg(a, opc_i);
    if m.e_objeto(opc) && m.tem(opc, ChaveProp::Nome(fixos::CAUSE))? {
        let c = m.obter(opc, ChaveProp::Nome(fixos::CAUSE))?;
        m.definir_propria(e, fixos::CAUSE, c, attr::METODO);
    }
    let pilha = m.texto_da_pilha();
    let cab = erro_to_string(m, e, Args { base: 0, n: 0 }, Valor::UNDEFINED)?;
    let s = m.concatenar(cab, pilha);
    m.definir_propria(e, fixos::STACK, s, attr::METODO);
    Ok(e)
}

fn erro_to_string(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    if !m.e_objeto(this) {
        return m.erro_tipo("Error.prototype.toString called on non-object");
    }
    let n = m.obter(this, ChaveProp::Nome(fixos::NAME))?;
    let n = if n == Valor::UNDEFINED { m.texto("Error") } else { m.para_texto(n)? };
    let msg = m.obter(this, ChaveProp::Nome(fixos::MESSAGE))?;
    let msg = if msg == Valor::UNDEFINED { Valor::VAZIA } else { m.para_texto(msg)? };
    if m.texto_len(n) == 0 {
        return Ok(msg);
    }
    if m.texto_len(msg) == 0 {
        return Ok(n);
    }
    let sep = m.texto(": ");
    let a = m.concatenar(n, sep);
    Ok(m.concatenar(a, msg))
}

// ---------------------------------------------------------------- iterador de array

const IT_VALORES: u32 = 0;
const IT_CHAVES: u32 = 1;
const IT_PARES: u32 = 2;

fn novo_iterador_array(m: &mut Motor, this: Valor, tipo: u32) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let p = m.intr.array_iterador_proto;
    let f = m.forma_para(Classe::Iterador, p, 0);
    let it = m.novo_objeto(f);
    m.por_interno_valor(it, 0, o);
    m.por_interno(it, 1, 0);
    m.por_interno(it, 2, tipo);
    Ok(it)
}

fn array_values(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    novo_iterador_array(m, this, IT_VALORES)
}

fn array_keys(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    novo_iterador_array(m, this, IT_CHAVES)
}

fn array_entries(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    novo_iterador_array(m, this, IT_PARES)
}

fn array_iterador_next(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    if !m.e_objeto(this) || m.formas.get(m.forma(this)).classe != Classe::Iterador {
        return m.erro_tipo("next method called on incompatible receiver");
    }
    let alvo = Valor(m.interno(this, 0));
    if alvo == Valor::UNDEFINED {
        return Ok(m.resultado_iterador(Valor::UNDEFINED, true));
    }
    let i = m.interno(this, 1);
    let n = m.comprimento_de(alvo)?;
    if u64::from(i) >= n {
        m.por_interno_valor(this, 0, Valor::UNDEFINED);
        return Ok(m.resultado_iterador(Valor::UNDEFINED, true));
    }
    m.por_interno(this, 1, i + 1);
    let v = match m.interno(this, 2) {
        IT_CHAVES => m.numero(f64::from(i)),
        IT_VALORES => m.obter_indice(alvo, u64::from(i))?,
        _ => {
            let x = m.obter_indice(alvo, u64::from(i))?;
            let k = m.numero(f64::from(i));
            m.array_de(&[k, x])
        }
    };
    Ok(m.resultado_iterador(v, false))
}

// ---------------------------------------------------------------- geradores

fn gerador_retomar(m: &mut Motor, this: Valor, ent: Entrada) -> Res<Valor> {
    if !m.e_objeto(this) || m.formas.get(m.forma(this)).classe != Classe::Gerador {
        return m.erro_tipo("next method called on incompatible receiver");
    }
    let id = m.interno(this, 0);
    if id == u32::MAX {
        return match ent {
            Entrada::Normal(_) => Ok(m.resultado_iterador(Valor::UNDEFINED, true)),
            Entrada::Lanca(e) => Err(e),
            Entrada::Retorna(v) => Ok(m.resultado_iterador(v, true)),
        };
    }
    match m.retomar(id, ent) {
        Ok(Fim::Suspenso(v)) => Ok(m.resultado_iterador(v, false)),
        Ok(Fim::Retorno(v)) => {
            m.liberar_suspenso(id);
            m.por_interno(this, 0, u32::MAX);
            Ok(m.resultado_iterador(v, true))
        }
        Err(e) => {
            if m.suspensos[id as usize].is_none() {
                m.liberar_suspenso(id);
                m.por_interno(this, 0, u32::MAX);
            }
            Err(e)
        }
    }
}

fn gerador_next(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    gerador_retomar(m, this, Entrada::Normal(v))
}

fn gerador_return(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    gerador_retomar(m, this, Entrada::Retorna(v))
}

fn gerador_throw(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    gerador_retomar(m, this, Entrada::Lanca(v))
}

// ---------------------------------------------------------------- console

fn juntar_args(m: &mut Motor, a: Args) -> Res<String> {
    let mut s = String::new();
    for i in 0..a.n {
        if i > 0 {
            s.push(' ');
        }
        let v = m.arg(a, i);
        let t = if m.e_objeto(v) && m.formas.get(m.forma(v)).classe == Classe::Erro {
            m.texto_do_erro(v)
        } else {
            m.inspecionar(v, 2)
        };
        s.push_str(&t);
    }
    Ok(s)
}

fn console_log(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let s = juntar_args(m, a)?;
    m.imprimir(0, &s);
    Ok(Valor::UNDEFINED)
}

fn console_info(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let s = juntar_args(m, a)?;
    m.imprimir(3, &s);
    Ok(Valor::UNDEFINED)
}

fn console_debug(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let s = juntar_args(m, a)?;
    m.imprimir(4, &s);
    Ok(Valor::UNDEFINED)
}

fn console_warn(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let s = juntar_args(m, a)?;
    m.imprimir(1, &s);
    Ok(Valor::UNDEFINED)
}

fn console_error(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let s = juntar_args(m, a)?;
    m.imprimir(2, &s);
    Ok(Valor::UNDEFINED)
}

impl Motor {
    /// Texto para o console (sem chamar código JS).
    pub fn inspecionar(&mut self, v: Valor, fundo: u32) -> String {
        if self.e_texto(v) {
            return self.texto_rust(v);
        }
        if self.e_bigint(v) {
            return format!("{}n", self.ler_bigint(v).para_texto(10));
        }
        if !self.e_objeto(v) {
            return self.para_texto_rust(v).unwrap_or_else(|_| "Symbol()".into());
        }
        if self.e_funcao(v) {
            let n = self.propria(v, fixos::NAME).map(|d| self.ler_campo(v, d.slot)).unwrap_or(Valor::VAZIA);
            let n = if self.e_texto(n) { self.texto_rust(n) } else { String::new() };
            return if n.is_empty() { "[Function (anônima)]".into() } else { format!("[Function: {n}]") };
        }
        if fundo == 0 {
            return if self.e_array(v) { "[Array]".into() } else { "[Object]".into() };
        }
        let mostrar = |m: &mut Motor, x: Valor| -> String {
            if m.e_texto(x) {
                format!("'{}'", m.texto_rust(x))
            } else {
                m.inspecionar(x, fundo - 1)
            }
        };
        if self.e_array(v) {
            let n = self.array_len(v);
            let mut partes = Vec::new();
            for i in 0..n.min(100) {
                let x = self.elemento(v, i).unwrap_or(Valor::UNDEFINED);
                partes.push(mostrar(self, x));
            }
            if n > 100 {
                partes.push(format!("... mais {} itens", n - 100));
            }
            return format!("[ {} ]", partes.join(", "));
        }
        let mut partes = Vec::new();
        for k in self.chaves_de(v, true, true, false).into_iter().take(50) {
            let nome = self.chave_para_rust(k);
            let x = match self.descritor_proprio(v, k) {
                Some(d) if d.acessor() => "[Getter/Setter]".into(),
                Some(d) => mostrar(self, d.valor),
                None => "undefined".into(),
            };
            partes.push(format!("{nome}: {x}"));
        }
        if partes.is_empty() {
            "{}".into()
        } else {
            format!("{{ {} }}", partes.join(", "))
        }
    }

}
