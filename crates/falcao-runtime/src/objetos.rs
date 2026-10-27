//! Objetos do JS na gaiola e as operações de propriedade sobre a forma.
//!
//! ```text
//!  [forma][extras][elementos][internos…][campos na linha…]
//! ```
//!
//! - Propriedade com `slot < inline` mora na linha; as outras em `extras` (uma lista que
//!   cresce dobrando).
//! - Itens indexados moram em `elementos` (lista), com o tipo na forma.
//! - Acessores (`get`/`set`) guardam um par `Tipo::Acessor` no campo; quem chama o getter é o
//!   intérprete (aqui só se diz que é acessor).

use crate::formas::{attr, Chave, Classe, Descritor, FormaId, TipoElementos};
use crate::motor::Motor;
use crate::valor::Valor;

/// Resultado de procurar uma propriedade na cadeia de protótipos.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Achado {
    /// Propriedade de dados, com o valor.
    Dado(Valor),
    /// Par de acessores (objeto `Tipo::Acessor`): o intérprete chama o getter.
    Acessor(Valor),
    Ausente,
}

impl Motor {
    // ---------------------------------------------------------------- criar

    /// Objeto novo com a forma dada (sem propriedades ainda).
    pub fn novo_objeto(&mut self, forma: FormaId) -> Valor {
        let f = self.formas.get(forma);
        let (n_int, mascara) = f.classe.internos();
        let inline = f.inline;
        let bytes = 12 + 4 * (n_int + inline);
        let d = self.g.alocar(bytes);
        self.g.por_u32(d, forma.0);
        self.g.por_u32(d + 4, Valor::LISTA_VAZIA.0);
        self.g.por_u32(d + 8, Valor::LISTA_VAZIA.0);
        for i in 0..n_int {
            let v = if mascara & (1 << i) != 0 { Valor::UNDEFINED.0 } else { 0 };
            self.g.por_u32(d + 12 + 4 * i, v);
        }
        for i in 0..inline {
            self.g.por_u32(d + 12 + 4 * (n_int + i), Valor::UNDEFINED.0);
        }
        Valor::de_deslocamento(d)
    }

    // ---------------------------------------------------------------- campos

    #[inline]
    fn inicio_campos(&self, forma: FormaId) -> u32 {
        12 + 4 * self.formas.get(forma).classe.internos().0
    }

    /// Campo interno `i` (cru, 32 bits).
    #[inline]
    pub fn interno(&self, obj: Valor, i: u32) -> u32 {
        self.g.u32_em(obj.deslocamento() + 12 + 4 * i)
    }

    #[inline]
    pub fn por_interno(&mut self, obj: Valor, i: u32, v: u32) {
        self.g.por_u32(obj.deslocamento() + 12 + 4 * i, v);
    }

    /// Campo interno `i` que guarda valor (com barreira).
    #[inline]
    pub fn por_interno_valor(&mut self, obj: Valor, i: u32, v: Valor) {
        let d = obj.deslocamento();
        self.g.por_valor(d, d + 12 + 4 * i, v);
    }

    #[inline]
    pub fn ler_campo(&self, obj: Valor, slot: u32) -> Valor {
        let forma = self.forma(obj);
        let inline = self.formas.get(forma).inline;
        let d = obj.deslocamento();
        if slot < inline {
            self.g.valor_em(d + self.inicio_campos(forma) + 4 * slot)
        } else {
            let extras = self.g.valor_em(d + 4);
            self.lista_get(extras, slot - inline)
        }
    }

    pub fn gravar_campo(&mut self, obj: Valor, slot: u32, v: Valor) {
        let forma = self.forma(obj);
        let inline = self.formas.get(forma).inline;
        let d = obj.deslocamento();
        if slot < inline {
            let off = d + self.inicio_campos(forma) + 4 * slot;
            self.g.por_valor(d, off, v);
            return;
        }
        let i = slot - inline;
        let mut extras = self.g.valor_em(d + 4);
        let cap = if extras == Valor::LISTA_VAZIA { 0 } else { self.lista_len(extras) };
        if i >= cap {
            let nova = (cap * 2).max(4).max(i + 1);
            extras = self.redimensionar_lista(extras, nova, Valor::UNDEFINED);
            self.g.por_valor(d, d + 4, extras);
        }
        self.lista_set(extras, i, v);
    }

    // ---------------------------------------------------------------- propriedades

    #[inline]
    pub fn propria(&self, obj: Valor, chave: Chave) -> Option<Descritor> {
        self.formas.procurar(self.forma(obj), chave)
    }

    #[inline]
    pub fn proto_de(&self, obj: Valor) -> Valor {
        self.formas.get(self.forma(obj)).proto
    }

    /// Procura `chave` no objeto e nos protótipos.
    pub fn procurar(&self, mut obj: Valor, chave: Chave) -> Achado {
        loop {
            if let Some(d) = self.propria(obj, chave) {
                let v = self.ler_campo(obj, d.slot);
                return if d.attrs & attr::ACESSOR != 0 { Achado::Acessor(v) } else { Achado::Dado(v) };
            }
            let p = self.proto_de(obj);
            if p == Valor::NULL {
                return Achado::Ausente;
            }
            obj = p;
        }
    }

    /// Acrescenta propriedade nova (o chamador já sabe que não existe).
    pub fn adicionar_propriedade(&mut self, obj: Valor, chave: Chave, v: Valor, attrs: u8) {
        let de = self.forma(obj);
        let h = self.hash_chave(chave);
        let para = self.formas.adicionar(de, chave, attrs, h);
        let slot = self.formas.procurar(para, chave).unwrap().slot;
        self.g.por_u32(obj.deslocamento(), para.0);
        self.gravar_campo(obj, slot, v);
    }

    /// `obj.chave = v` só no próprio objeto, criando com atributos padrão se não existe.
    /// Devolve falso se a propriedade existe e não é gravável (ou o objeto está fechado).
    pub fn gravar_propria(&mut self, obj: Valor, chave: Chave, v: Valor) -> bool {
        match self.propria(obj, chave) {
            Some(d) if d.attrs & attr::ACESSOR != 0 => false,
            Some(d) if d.attrs & attr::GRAVAVEL == 0 => false,
            Some(d) => {
                self.gravar_campo(obj, d.slot, v);
                true
            }
            None if !self.formas.get(self.forma(obj)).extensivel => false,
            None => {
                self.adicionar_propriedade(obj, chave, v, attr::PADRAO);
                true
            }
        }
    }

    /// Define (ou redefine) a propriedade com valor e atributos, sem conferir nada
    /// (`CreateDataProperty`/`DefineOwnProperty` já validados por quem chama).
    pub fn definir_propria(&mut self, obj: Valor, chave: Chave, v: Valor, attrs: u8) {
        match self.propria(obj, chave) {
            Some(d) => {
                if d.attrs != attrs {
                    let nova = self.formas.mudar_atributos(self.forma(obj), chave, attrs);
                    self.g.por_u32(obj.deslocamento(), nova.0);
                }
                self.gravar_campo(obj, d.slot, v);
            }
            None => self.adicionar_propriedade(obj, chave, v, attrs),
        }
    }

    /// `delete obj.chave` no próprio objeto. Falso se não é configurável.
    pub fn apagar_propria(&mut self, obj: Valor, chave: Chave) -> bool {
        let Some(d) = self.propria(obj, chave) else { return true };
        if d.attrs & attr::CONFIGURAVEL == 0 {
            return false;
        }
        let dic = self.formas.para_dicionario(self.forma(obj));
        self.g.por_u32(obj.deslocamento(), dic.0);
        self.formas.remover(dic, chave);
        self.gravar_campo(obj, d.slot, Valor::UNDEFINED);
        true
    }

    /// Chaves próprias na ordem de criação (sem os índices).
    pub fn chaves_proprias(&self, obj: Valor) -> Vec<Descritor> {
        self.formas.descritores(self.forma(obj)).to_vec()
    }

    pub fn definir_proto(&mut self, obj: Valor, proto: Valor) {
        let de = self.forma(obj);
        let f = self.formas.get(de);
        if f.proto == proto {
            return;
        }
        let (classe, inline, elementos) = (f.classe, f.inline, f.elementos);
        if proto != Valor::NULL {
            let fp = self.forma(proto);
            self.formas.get_mut(fp).e_prototipo = true;
        }
        let hp = if proto == Valor::NULL { 0 } else { self.formas.get(self.forma(proto)).hash };
        // Refaz o caminho de transições sobre a raiz do protótipo novo (mesmos slots).
        let descr = self.chaves_proprias(obj);
        let mut nova = self.formas.raiz(classe, proto, inline, elementos, hp);
        for d in &descr {
            let h = self.hash_chave(d.chave);
            nova = self.formas.adicionar(nova, d.chave, d.attrs, h);
        }
        let ok = descr.iter().all(|d| self.formas.procurar(nova, d.chave).map(|x| x.slot) == Some(d.slot));
        if !ok {
            // Dicionário com campos vagos: os slots não batem; copia os valores para os novos.
            let valores: Vec<Valor> = descr.iter().map(|d| self.ler_campo(obj, d.slot)).collect();
            self.g.por_u32(obj.deslocamento(), nova.0);
            for (d, v) in descr.iter().zip(valores) {
                let s = self.formas.procurar(nova, d.chave).unwrap().slot;
                self.gravar_campo(obj, s, v);
            }
            return;
        }
        if !self.formas.get(de).extensivel {
            nova = self.formas.fechar(nova);
        }
        self.g.por_u32(obj.deslocamento(), nova.0);
    }

    // ---------------------------------------------------------------- elementos

    #[inline]
    pub fn elementos(&self, obj: Valor) -> Valor {
        self.g.valor_em(obj.deslocamento() + 8)
    }

    #[inline]
    pub fn e_array(&self, v: Valor) -> bool {
        self.e_objeto(v) && self.formas.get(self.forma(v)).classe == Classe::Array
    }

    /// `length` de array (campo interno 0).
    #[inline]
    pub fn array_len(&self, a: Valor) -> u32 {
        self.interno(a, 0)
    }

    /// Item `i` próprio, se existe (buraco = ausente).
    pub fn elemento(&self, obj: Valor, i: u32) -> Option<Valor> {
        let el = self.elementos(obj);
        if el == Valor::LISTA_VAZIA || i >= self.lista_len(el) {
            return None;
        }
        let v = self.lista_get(el, i);
        (v != Valor::BURACO).then_some(v)
    }

    /// Grava o item `i` (estica a lista; atualiza `length` de array). Falso se o objeto está
    /// fechado e o item não existe.
    pub fn gravar_elemento(&mut self, obj: Valor, i: u32, v: Valor) -> bool {
        let d = obj.deslocamento();
        let mut el = self.elementos(obj);
        let cap = if el == Valor::LISTA_VAZIA { 0 } else { self.lista_len(el) };
        let forma = self.forma(obj);
        let f = self.formas.get(forma);
        let (tipo, extensivel, e_array) = (f.elementos, f.extensivel, f.classe == Classe::Array);
        let existe = i < cap && self.lista_get(el, i) != Valor::BURACO;
        if !existe && !extensivel {
            return false;
        }
        let len = if e_array { self.array_len(obj) } else { cap };
        let mut novo_tipo = tipo;
        if !existe && self.indices_especiais {
            // Já guardado com nome (esparso, ou com atributos próprios): continua lá.
            let c = self.nome_do_indice(i);
            if let Some(d) = self.propria(obj, c) {
                if d.attrs & (attr::ACESSOR | attr::GRAVAVEL) != attr::GRAVAVEL {
                    return false;
                }
                self.gravar_campo(obj, d.slot, v);
                return true;
            }
        }
        if i >= cap && i > cap.saturating_mul(2).saturating_add(1024) {
            // Muito além do fim (`x[175237375] = 1`): vira propriedade com nome, sem esticar a lista.
            self.indices_especiais = true;
            let c = self.nome_do_indice(i);
            self.definir_propria(obj, c, v, attr::PADRAO);
            if e_array && i >= len {
                self.por_interno(obj, 0, i + 1);
            }
            return true;
        }
        if i >= cap {
            let nova = (cap + cap / 2 + 4).max(i + 1);
            el = self.redimensionar_lista(el, nova, Valor::BURACO);
            self.g.por_valor(d, d + 8, el);
        }
        if i > len {
            novo_tipo = com_buracos(novo_tipo);
        }
        if !v.e_int() && matches!(novo_tipo, TipoElementos::Inteiros | TipoElementos::InteirosComBuracos) {
            novo_tipo = if novo_tipo.com_buracos() { TipoElementos::ValoresComBuracos } else { TipoElementos::Valores };
        }
        if novo_tipo != tipo {
            let nf = self.formas.com_elementos(forma, novo_tipo);
            self.g.por_u32(d, nf.0);
        }
        self.lista_set(el, i, v);
        if e_array && i >= len {
            self.por_interno(obj, 0, i + 1);
        }
        true
    }

    pub fn apagar_elemento(&mut self, obj: Valor, i: u32) -> bool {
        let el = self.elementos(obj);
        if el == Valor::LISTA_VAZIA || i >= self.lista_len(el) {
            return true;
        }
        if !self.formas.get(self.forma(obj)).extensivel {
            // shortcut: objeto selado/congelado não apaga item; o certo é olhar os atributos
            // de cada item (só existem no modo dicionário de elementos).
            return false;
        }
        let forma = self.forma(obj);
        let t = com_buracos(self.formas.get(forma).elementos);
        let nf = self.formas.com_elementos(forma, t);
        self.g.por_u32(obj.deslocamento(), nf.0);
        self.lista_set(el, i, Valor::BURACO);
        true
    }

    /// Muda o `length` de array (cortar apaga os itens do fim).
    pub fn definir_array_len(&mut self, a: Valor, n: u32) {
        let len = self.array_len(a);
        if n < len {
            let el = self.elementos(a);
            let cap = if el == Valor::LISTA_VAZIA { 0 } else { self.lista_len(el) };
            for i in n..len.min(cap) {
                self.lista_set(el, i, Valor::BURACO);
            }
        } else if n > len {
            let forma = self.forma(a);
            let t = com_buracos(self.formas.get(forma).elementos);
            let nf = self.formas.com_elementos(forma, t);
            self.g.por_u32(a.deslocamento(), nf.0);
        }
        self.por_interno(a, 0, n);
    }
}

fn com_buracos(t: TipoElementos) -> TipoElementos {
    match t {
        TipoElementos::Inteiros => TipoElementos::InteirosComBuracos,
        TipoElementos::Doubles => TipoElementos::DoublesComBuracos,
        TipoElementos::Valores => TipoElementos::ValoresComBuracos,
        x => x,
    }
}

#[cfg(test)]
mod testes {
    use super::*;
    use crate::nomes::fixos;

    #[test]
    fn propriedades_protos_e_elementos() {
        let mut m = Motor::novo();
        let raiz = m.formas.raiz(Classe::Comum, Valor::NULL, 2, TipoElementos::Inteiros, 1);
        let proto = m.novo_objeto(raiz);
        let x = m.nomes.internar_str("x");
        let y = m.nomes.internar_str("y");
        let z = m.nomes.internar_str("z");
        m.gravar_propria(proto, z, Valor::int(9));
        let hp = m.formas.get(m.forma(proto)).hash;
        let rf = m.formas.raiz(Classe::Comum, proto, 2, TipoElementos::Inteiros, hp);
        let a = m.novo_objeto(rf);
        let b = m.novo_objeto(rf);
        for o in [a, b] {
            m.gravar_propria(o, x, Valor::int(1));
            m.gravar_propria(o, y, Valor::int(2));
        }
        assert_eq!(m.forma(a), m.forma(b), "mesmo caminho, mesma forma");
        // Terceira propriedade passa da linha (2) e vai para extras.
        m.gravar_propria(a, fixos::NAME, Valor::int(3));
        assert_eq!(m.procurar(a, fixos::NAME), Achado::Dado(Valor::int(3)));
        assert_eq!(m.procurar(a, z), Achado::Dado(Valor::int(9)), "veio do protótipo");
        assert_eq!(m.procurar(a, fixos::LENGTH), Achado::Ausente);
        assert!(m.apagar_propria(a, x));
        assert_eq!(m.procurar(a, x), Achado::Ausente);
        assert_eq!(m.procurar(a, y), Achado::Dado(Valor::int(2)));
        assert_eq!(m.procurar(b, x), Achado::Dado(Valor::int(1)), "b não muda");
        // Array: length e tipos de elementos.
        let ra = m.formas.raiz(Classe::Array, Valor::NULL, 0, TipoElementos::Inteiros, 2);
        let arr = m.novo_objeto(ra);
        for i in 0..10 {
            m.gravar_elemento(arr, i, Valor::int(i as i32));
        }
        assert_eq!(m.array_len(arr), 10);
        assert_eq!(m.formas.get(m.forma(arr)).elementos, TipoElementos::Inteiros);
        m.gravar_elemento(arr, 20, Valor::TRUE);
        assert_eq!(m.array_len(arr), 21);
        assert_eq!(m.formas.get(m.forma(arr)).elementos, TipoElementos::ValoresComBuracos);
        assert_eq!(m.elemento(arr, 15), None);
        m.definir_array_len(arr, 5);
        assert_eq!(m.elemento(arr, 7), None);
        assert_eq!(m.elemento(arr, 4), Some(Valor::int(4)));
        // Trocar o protótipo mantém as propriedades.
        m.definir_proto(b, Valor::NULL);
        assert_eq!(m.procurar(b, y), Achado::Dado(Valor::int(2)));
        assert_eq!(m.procurar(b, z), Achado::Ausente);
    }
}
