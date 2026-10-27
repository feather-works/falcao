//! Objetos intrínsecos (protótipos da linguagem, objeto global) e as formas raiz mais usadas.
//! Aqui só nascem os objetos; os métodos são postos pelos módulos de `builtins`.

use crate::formas::{attr, Chave, Classe, FormaId, TipoElementos};
use crate::motor::Motor;
use crate::nomes::fixos;
use crate::valor::Valor;

/// Símbolos bem conhecidos (`Symbol.iterator`…), com número fixo.
pub mod simb {
    use crate::formas::Chave;
    pub const ITERATOR: Chave = Chave(Chave::BIT_SIMBOLO);
    pub const ASYNC_ITERATOR: Chave = Chave(Chave::BIT_SIMBOLO | 1);
    pub const HAS_INSTANCE: Chave = Chave(Chave::BIT_SIMBOLO | 2);
    pub const TO_PRIMITIVE: Chave = Chave(Chave::BIT_SIMBOLO | 3);
    pub const TO_STRING_TAG: Chave = Chave(Chave::BIT_SIMBOLO | 4);
    pub const SPECIES: Chave = Chave(Chave::BIT_SIMBOLO | 5);
    pub const IS_CONCAT_SPREADABLE: Chave = Chave(Chave::BIT_SIMBOLO | 6);
    pub const UNSCOPABLES: Chave = Chave(Chave::BIT_SIMBOLO | 7);
    pub const MATCH: Chave = Chave(Chave::BIT_SIMBOLO | 8);
    pub const MATCH_ALL: Chave = Chave(Chave::BIT_SIMBOLO | 9);
    pub const REPLACE: Chave = Chave(Chave::BIT_SIMBOLO | 10);
    pub const SEARCH: Chave = Chave(Chave::BIT_SIMBOLO | 11);
    pub const SPLIT: Chave = Chave(Chave::BIT_SIMBOLO | 12);
    /// Lista de campos de instância de uma classe (interno, escondido da reflexão).
    pub const CAMPOS: Chave = Chave(Chave::BIT_SIMBOLO | 13);
    pub const NOMES: [&str; 14] = [
        "Symbol.iterator", "Symbol.asyncIterator", "Symbol.hasInstance", "Symbol.toPrimitive",
        "Symbol.toStringTag", "Symbol.species", "Symbol.isConcatSpreadable", "Symbol.unscopables",
        "Symbol.match", "Symbol.matchAll", "Symbol.replace", "Symbol.search", "Symbol.split", "campos",
    ];
    /// Símbolos até aqui (exclusive) são públicos; daqui em diante, internos.
    pub const PUBLICOS: u32 = 13;
}

/// Tipos de erro nativos.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TipoErro {
    Erro,
    Tipo,
    Referencia,
    Sintaxe,
    Alcance,
    Uri,
    Eval,
    Agregado,
}

pub struct Intrinsecos {
    pub global: Valor,
    pub objeto_proto: Valor,
    pub funcao_proto: Valor,
    pub array_proto: Valor,
    pub texto_proto: Valor,
    pub numero_proto: Valor,
    pub booleano_proto: Valor,
    pub simbolo_proto: Valor,
    pub bigint_proto: Valor,
    pub erro_protos: [Valor; 8],
    pub iterador_proto: Valor,
    pub iterador_helper_proto: Valor,
    pub wrap_iterador_proto: Valor,
    pub array_buffer_proto: Valor,
    /// Protótipos de Int8Array…Float64Array, na ordem de `builtins::tipados::TIPOS`.
    pub tipados_protos: [Valor; 12],
    /// Os construtores originais (Int8Array…), na mesma ordem.
    pub tipados_ctors: [Valor; 12],
    pub shared_array_buffer_proto: Valor,
    /// Esperas de `Atomics.waitAsync`: array de trincas [buffer, byte, promessa].
    pub atomics_esperas: Valor,
    pub array_iterador_proto: Valor,
    pub gerador_funcao_proto: Valor,
    pub gerador_proto: Valor,
    /// %AsyncFunction.prototype%, %AsyncGeneratorFunction.prototype%, %AsyncGeneratorPrototype%,
    /// %AsyncIteratorPrototype% e %AsyncFromSyncIteratorPrototype%.
    pub async_funcao_proto: Valor,
    pub async_gerador_funcao_proto: Valor,
    pub async_gerador_proto: Valor,
    pub async_iterador_proto: Valor,
    pub async_de_sync_proto: Valor,
    pub forma_objeto: FormaId,
    pub forma_objeto_vazio: FormaId,
    pub forma_array: FormaId,
    pub forma_funcao: FormaId,
    pub forma_funcao_simples: FormaId,
    pub forma_funcao_gerador: FormaId,
    pub forma_funcao_gerador_async: FormaId,
    pub forma_funcao_async: FormaId,
    pub forma_objeto_gerador: FormaId,
    pub forma_objeto_gerador_async: FormaId,
    pub formas_erro: [FormaId; 8],
    pub forma_argumentos: FormaId,
    pub promessa_proto: Valor,
    /// %Promise% (o construtor original).
    pub promessa_ctor: Valor,
    /// As partes de Promise escritas em JS (lista de funções), feitas no primeiro uso.
    pub promessa_js: Valor,
    pub regex_proto: Valor,
    pub regex_exec: Valor,
    pub eval: Valor,
    pub array_valores: Valor,
    pub array_iterador_next: Valor,
    pub funcao_has_instance: Valor,
    pub nativa_classe_base: u32,
    pub nativa_classe_derivada: u32,
    pub nativa_resolver: u32,
    pub nativa_rejeitar: u32,
}

impl Intrinsecos {
    /// Valores provisórios até [`Motor::iniciar_intrinsecos`] rodar.
    pub fn vazios() -> Self {
        let f = FormaId(0);
        let u = Valor::UNDEFINED;
        Self {
            global: u,
            objeto_proto: u,
            funcao_proto: u,
            array_proto: u,
            texto_proto: u,
            numero_proto: u,
            booleano_proto: u,
            simbolo_proto: u,
            array_buffer_proto: u,
            tipados_protos: [u; 12],
            tipados_ctors: [u; 12],
            shared_array_buffer_proto: u,
            atomics_esperas: u,
            bigint_proto: u,
            erro_protos: [u; 8],
            iterador_proto: u,
            iterador_helper_proto: u,
            wrap_iterador_proto: u,
            array_iterador_proto: u,
            gerador_funcao_proto: u,
            gerador_proto: u,
            async_funcao_proto: u,
            async_gerador_funcao_proto: u,
            async_gerador_proto: u,
            async_iterador_proto: u,
            async_de_sync_proto: u,
            forma_objeto: f,
            forma_objeto_vazio: f,
            forma_array: f,
            forma_funcao: f,
            forma_funcao_simples: f,
            forma_funcao_gerador: f,
            forma_funcao_gerador_async: f,
            forma_funcao_async: f,
            forma_objeto_gerador: f,
            forma_objeto_gerador_async: f,
            formas_erro: [f; 8],
            forma_argumentos: f,
            promessa_proto: u,
            promessa_ctor: u,
            promessa_js: u,
            regex_proto: u,
            regex_exec: u,
            eval: u,
            array_valores: u,
            array_iterador_next: u,
            funcao_has_instance: u,
            nativa_classe_base: 0,
            nativa_classe_derivada: 0,
            nativa_resolver: 0,
            nativa_rejeitar: 0,
        }
    }

    /// Todos os valores guardados aqui (raízes do coletor).
    pub fn valores_mut(&mut self) -> Vec<&mut Valor> {
        let mut v: Vec<&mut Valor> = vec![
            &mut self.global,
            &mut self.objeto_proto,
            &mut self.funcao_proto,
            &mut self.array_proto,
            &mut self.texto_proto,
            &mut self.numero_proto,
            &mut self.booleano_proto,
            &mut self.simbolo_proto,
            &mut self.bigint_proto,
            &mut self.iterador_proto,
            &mut self.iterador_helper_proto,
            &mut self.wrap_iterador_proto,
            &mut self.array_iterador_proto,
            &mut self.gerador_funcao_proto,
            &mut self.gerador_proto,
            &mut self.async_funcao_proto,
            &mut self.async_gerador_funcao_proto,
            &mut self.async_gerador_proto,
            &mut self.async_iterador_proto,
            &mut self.async_de_sync_proto,
            &mut self.promessa_proto,
            &mut self.promessa_ctor,
            &mut self.promessa_js,
            &mut self.regex_proto,
            &mut self.regex_exec,
            &mut self.eval,
            &mut self.array_valores,
            &mut self.array_iterador_next,
            &mut self.funcao_has_instance,
        ];
        v.push(&mut self.array_buffer_proto);
        v.push(&mut self.shared_array_buffer_proto);
        v.push(&mut self.atomics_esperas);
        v.extend(self.erro_protos.iter_mut());
        v.extend(self.tipados_protos.iter_mut());
        v.extend(self.tipados_ctors.iter_mut());
        v
    }
}

/// Função que não faz nada (`Function.prototype` é chamável).
fn nada(_: &mut Motor, _: Valor, _: crate::funcoes::Args, _: Valor) -> crate::funcoes::Res<Valor> {
    Ok(Valor::UNDEFINED)
}

impl Motor {
    /// Objeto comum vazio sem protótipo próprio de classe (`{}`).
    pub fn objeto_vazio(&mut self) -> Valor {
        let f = self.intr.forma_objeto;
        self.novo_objeto(f)
    }

    fn prototipo(&mut self, classe: Classe, proto: Valor) -> Valor {
        let f = self.forma_para(classe, proto, 0);
        let o = self.novo_objeto(f);
        let fo = self.forma(o);
        self.formas.get_mut(fo).e_prototipo = true;
        o
    }

    /// Cria os protótipos, as formas raiz e o objeto global.
    pub(crate) fn iniciar_intrinsecos(&mut self) {
        // Símbolos bem conhecidos, na ordem de `simb`.
        for nome in simb::NOMES {
            let d = self.texto(nome);
            self.novo_simbolo(d);
        }
        let op = self.prototipo(Classe::Comum, Valor::NULL);
        self.intr.objeto_proto = op;
        // Function.prototype é uma função (nativa que devolve undefined).
        let fp_forma = self.forma_para(Classe::Funcao, op, 0);
        let fp = self.novo_objeto(fp_forma);
        let id = self.nativas.len() as u32;
        self.nativas.push(crate::funcoes::Nativa { f: nada, nome: "", construtor: false });
        self.por_interno(fp, crate::funcoes::F_CODIGO, id | crate::funcoes::BIT_NATIVA);
        self.intr.funcao_proto = fp;
        // Formas de função: length e name (e prototype preguiçoso nas que constroem).
        let comp = attr::CONFIGURAVEL;
        let base = self.forma_para(Classe::Funcao, fp, 3);
        let h_len = self.hash_chave(fixos::LENGTH);
        let h_nome = self.hash_chave(fixos::NAME);
        let h_proto = self.hash_chave(fixos::PROTOTYPE);
        let a = self.formas.adicionar(base, fixos::LENGTH, comp, h_len);
        let simples = self.formas.adicionar(a, fixos::NAME, comp, h_nome);
        self.intr.forma_funcao_simples = simples;
        self.intr.forma_funcao = self.formas.adicionar(simples, fixos::PROTOTYPE, attr::GRAVAVEL | attr::PREGUICOSO, h_proto);
        self.definir_propria(fp, fixos::LENGTH, Valor::int(0), attr::CONFIGURAVEL);
        self.definir_propria(fp, fixos::NAME, Valor::VAZIA, attr::CONFIGURAVEL);
        // Objetos comuns.
        self.intr.forma_objeto = self.forma_para(Classe::Comum, op, 4);
        self.intr.forma_objeto_vazio = self.forma_para(Classe::Comum, op, 0);
        // Arrays (Array.prototype também é array).
        let ap_forma = self.forma_para(Classe::Array, op, 0);
        let ap = self.novo_objeto(ap_forma);
        self.formas.get_mut(ap_forma).e_prototipo = true;
        self.intr.array_proto = ap;
        self.intr.forma_array = self.formas.raiz(Classe::Array, ap, 0, TipoElementos::Inteiros, 77);
        // Primitivos.
        self.intr.texto_proto = self.prototipo(Classe::Caixa, op);
        self.por_interno_valor(self.intr.texto_proto, 0, Valor::VAZIA);
        self.intr.numero_proto = self.prototipo(Classe::Caixa, op);
        self.por_interno_valor(self.intr.numero_proto, 0, Valor::int(0));
        self.intr.booleano_proto = self.prototipo(Classe::Caixa, op);
        self.por_interno_valor(self.intr.booleano_proto, 0, Valor::FALSE);
        self.intr.simbolo_proto = self.prototipo(Classe::Comum, op);
        self.intr.bigint_proto = self.prototipo(Classe::Comum, op);
        // Erros: Error.prototype e os derivados.
        let ep = self.prototipo(Classe::Comum, op);
        self.intr.erro_protos[0] = ep;
        for i in 1..8 {
            self.intr.erro_protos[i] = self.prototipo(Classe::Comum, ep);
        }
        for i in 0..8 {
            let p = self.intr.erro_protos[i];
            self.intr.formas_erro[i] = self.forma_para(Classe::Erro, p, 2);
        }
        // Iteradores e geradores.
        let ip = self.prototipo(Classe::Comum, op);
        self.intr.iterador_proto = ip;
        self.intr.array_iterador_proto = self.prototipo(Classe::Comum, ip);
        let gfp = self.prototipo(Classe::Comum, fp);
        self.intr.gerador_funcao_proto = gfp;
        let gp = self.prototipo(Classe::Comum, ip);
        self.intr.gerador_proto = gp;
        let gbase = self.forma_para(Classe::Funcao, gfp, 3);
        let a = self.formas.adicionar(gbase, fixos::LENGTH, comp, h_len);
        let b = self.formas.adicionar(a, fixos::NAME, comp, h_nome);
        self.intr.forma_funcao_gerador = self.formas.adicionar(b, fixos::PROTOTYPE, attr::GRAVAVEL | attr::PREGUICOSO, h_proto);
        // O `prototype` de uma função geradora é objeto comum (não um gerador sem quadro).
        self.intr.forma_objeto_gerador = self.forma_para(Classe::Comum, gp, 0);
        // Async: funções, geradores e iteradores.
        let aip = self.prototipo(Classe::Comum, op);
        self.intr.async_iterador_proto = aip;
        self.intr.async_de_sync_proto = self.prototipo(Classe::Comum, aip);
        let afp = self.prototipo(Classe::Comum, fp);
        self.intr.async_funcao_proto = afp;
        let abase = self.forma_para(Classe::Funcao, afp, 3);
        let a = self.formas.adicionar(abase, fixos::LENGTH, comp, h_len);
        self.intr.forma_funcao_async = self.formas.adicionar(a, fixos::NAME, comp, h_nome);
        let agfp = self.prototipo(Classe::Comum, fp);
        self.intr.async_gerador_funcao_proto = agfp;
        let agp = self.prototipo(Classe::Comum, aip);
        self.intr.async_gerador_proto = agp;
        let agbase = self.forma_para(Classe::Funcao, agfp, 3);
        let a = self.formas.adicionar(agbase, fixos::LENGTH, comp, h_len);
        let b = self.formas.adicionar(a, fixos::NAME, comp, h_nome);
        self.intr.forma_funcao_gerador_async = self.formas.adicionar(b, fixos::PROTOTYPE, attr::GRAVAVEL | attr::PREGUICOSO, h_proto);
        self.intr.forma_objeto_gerador_async = self.forma_para(Classe::Comum, agp, 0);
        self.intr.forma_argumentos = self.forma_para(Classe::Argumentos, op, 0);
        // Objeto global.
        let gforma = self.forma_para(Classe::Comum, op, 0);
        let global = self.novo_objeto(gforma);
        self.intr.global = global;
        self.definir_propria(global, fixos::GLOBAL_THIS, global, attr::METODO);
        let nan = self.caixa_numero(f64::NAN);
        self.definir_propria(global, fixos::NAN, nan, 0);
        let inf = self.caixa_numero(f64::INFINITY);
        self.definir_propria(global, fixos::INFINITY, inf, 0);
        self.definir_propria(global, fixos::UNDEFINED, Valor::UNDEFINED, 0);
    }

    /// Erro novo do tipo dado com a mensagem.
    pub fn erro(&mut self, tipo: TipoErro, msg: &str) -> Valor {
        let m = self.texto(msg);
        self.erro_com(tipo, m)
    }

    pub fn erro_com(&mut self, tipo: TipoErro, msg: Valor) -> Valor {
        let forma = self.intr.formas_erro[tipo as usize];
        let e = self.novo_objeto(forma);
        if msg != Valor::UNDEFINED {
            self.definir_propria(e, fixos::MESSAGE, msg, attr::METODO);
        }
        let pilha = self.texto_da_pilha();
        self.definir_propria(e, fixos::STACK, pilha, attr::METODO);
        self.erro_interno = (e, self.quadros.len() as u32);
        e
    }

    /// `Err(TypeError(msg))`, para usar com `?`.
    pub fn erro_tipo<T>(&mut self, msg: &str) -> crate::funcoes::Res<T> {
        Err(self.erro(TipoErro::Tipo, msg))
    }

    pub fn erro_alcance<T>(&mut self, msg: &str) -> crate::funcoes::Res<T> {
        Err(self.erro(TipoErro::Alcance, msg))
    }

    pub fn erro_referencia<T>(&mut self, msg: &str) -> crate::funcoes::Res<T> {
        Err(self.erro(TipoErro::Referencia, msg))
    }

    /// Chave de propriedade pelo texto (cria o nome se não existe).
    pub fn chave(&mut self, s: &str) -> Chave {
        self.nomes.internar_str(s)
    }
}
