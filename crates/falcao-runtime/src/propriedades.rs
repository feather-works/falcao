//! Operações de propriedade da linguagem (§10.1 e §7.3): `[[Get]]`, `[[Set]]`, `[[HasProperty]]`,
//! `[[Delete]]`, `[[DefineOwnProperty]]` e `[[OwnPropertyKeys]]`, com getters/setters, os itens
//! indexados, `length` de array, textos e o `prototype` preguiçoso das funções.
//!
//! Item indexado com atributos fora do padrão (acessor, só leitura) mora como nome comum (o
//! texto do índice); [`Motor::indices_especiais`] liga a busca por esse caminho.

use crate::formas::{self, attr, Chave, Classe};
use crate::funcoes::Res;
use crate::intrinsecos::simb;
use crate::motor::Motor;
use crate::nomes::fixos;
use crate::textos::ChaveProp;
use crate::valor::Valor;

/// Descritor completo de propriedade própria.
#[derive(Clone, Copy, Debug)]
pub struct DescProp {
    pub valor: Valor,
    pub get: Valor,
    pub set: Valor,
    pub attrs: u8,
}

impl DescProp {
    pub fn acessor(&self) -> bool {
        self.attrs & attr::ACESSOR != 0
    }
}

/// Descritor parcial (`Object.defineProperty`).
#[derive(Clone, Copy, Debug, Default)]
pub struct DescParcial {
    pub valor: Option<Valor>,
    pub get: Option<Valor>,
    pub set: Option<Valor>,
    pub gravavel: Option<bool>,
    pub enumeravel: Option<bool>,
    pub configuravel: Option<bool>,
}

impl DescParcial {
    pub fn e_acessor(&self) -> bool {
        self.get.is_some() || self.set.is_some()
    }
}

impl Motor {
    // ---------------------------------------------------------------- pares de acessores

    pub fn novo_acessor(&mut self, get: Valor, set: Valor) -> Valor {
        let d = self.g.alocar(12);
        self.g.por_u32(d, formas::ACESSOR.0);
        self.g.por_valor(d, d + 4, get);
        self.g.por_valor(d, d + 8, set);
        Valor::de_deslocamento(d)
    }

    #[inline]
    pub fn acessor_get(&self, p: Valor) -> Valor {
        self.g.valor_em(p.deslocamento() + 4)
    }

    #[inline]
    pub fn acessor_set(&self, p: Valor) -> Valor {
        self.g.valor_em(p.deslocamento() + 8)
    }

    // ---------------------------------------------------------------- chaves

    /// Nome comum que guarda um item indexado especial.
    pub(crate) fn nome_do_indice(&mut self, i: u32) -> Chave {
        self.nomes.internar_str(&i.to_string())
    }

    /// Texto primitivo dentro de uma caixa de String, se for o caso.
    #[inline]
    fn texto_da_caixa(&self, o: Valor) -> Option<Valor> {
        if self.formas.get(self.forma(o)).classe != Classe::Caixa {
            return None;
        }
        let v = Valor(self.interno(o, 0));
        self.e_texto(v).then_some(v)
    }

    // ---------------------------------------------------------------- próprias

    /// Descritor próprio (`[[GetOwnProperty]]`).
    pub fn descritor_proprio(&mut self, o: Valor, k: ChaveProp) -> Option<DescProp> {
        let dado = |v: Valor, attrs: u8| Some(DescProp { valor: v, get: Valor::UNDEFINED, set: Valor::UNDEFINED, attrs });
        match k {
            ChaveProp::Indice(i) => {
                if self.formas.get(self.forma(o)).classe == Classe::ArrayTipado {
                    return self.tipado_ler(o, i).and_then(|v| dado(v, attr::PADRAO));
                }
                if let Some(v) = self.elemento(o, i) {
                    return dado(v, attr::PADRAO);
                }
                if let Some(t) = self.texto_da_caixa(o) {
                    if i < self.texto_len(t) {
                        let c = self.unidade_em(t, i);
                        let s = self.texto_u16(&[c]);
                        return dado(s, attr::ENUMERAVEL);
                    }
                }
                if self.indices_especiais {
                    let c = self.nome_do_indice(i);
                    return self.descritor_proprio(o, ChaveProp::Nome(c));
                }
                None
            }
            ChaveProp::Nome(c) => {
                let classe = self.formas.get(self.forma(o)).classe;
                if classe == Classe::ArrayTipado && self.numerico_canonico(c) {
                    return None;
                }
                if c == fixos::LENGTH {
                    if classe == Classe::Array {
                        let n = self.array_len(o);
                        let v = self.numero(f64::from(n));
                        let a = if self.formas.get(self.forma(o)).extensivel || self.propria(o, c).is_none() { attr::GRAVAVEL } else { 0 };
                        return dado(v, a);
                    }
                    if let Some(t) = self.texto_da_caixa(o) {
                        return dado(Valor::int(self.texto_len(t) as i32), 0);
                    }
                }
                let d = self.propria(o, c)?;
                let v = self.ler_campo(o, d.slot);
                if d.attrs & attr::ACESSOR != 0 {
                    return Some(DescProp { valor: Valor::UNDEFINED, get: self.acessor_get(v), set: self.acessor_set(v), attrs: d.attrs });
                }
                let v = if v == Valor::BURACO && d.attrs & attr::PREGUICOSO != 0 { self.materializar_prototype(o) } else { v };
                dado(v, d.attrs & !attr::PREGUICOSO)
            }
        }
    }

    // ---------------------------------------------------------------- [[Get]]

    /// `alvo[k]`, com `alvo` objeto ou primitivo.
    pub fn obter(&mut self, alvo: Valor, k: ChaveProp) -> Res<Valor> {
        if self.e_objeto(alvo) {
            return self.obter_com(alvo, k, alvo);
        }
        if self.e_texto(alvo) {
            match k {
                ChaveProp::Nome(fixos::LENGTH) => return Ok(Valor::int(self.texto_len(alvo) as i32)),
                ChaveProp::Indice(i) if i < self.texto_len(alvo) => {
                    let c = self.unidade_em(alvo, i);
                    return Ok(if c < 256 { self.caracteres[c as usize] } else { self.texto_u16(&[c]) });
                }
                _ => {}
            }
        }
        if alvo.e_nulo_ou_indefinido() || alvo == Valor::BURACO {
            let nome = self.chave_para_rust(k);
            let t = if alvo == Valor::NULL { "null" } else { "undefined" };
            return self.erro_tipo(&format!("Cannot read properties of {t} (reading '{nome}')"));
        }
        let proto = self.proto_do_primitivo(alvo);
        self.obter_com(proto, k, alvo)
    }

    /// `[[Get]]` de `o` com o receptor dado (para getters).
    pub fn obter_com(&mut self, mut o: Valor, k: ChaveProp, receptor: Valor) -> Res<Valor> {
        loop {
            let classe = self.formas.get(self.forma(o)).classe;
            if classe == Classe::Proxy {
                return self.proxy_obter(o, k, receptor);
            }
            // Chave numérica de array tipado: o item ou undefined, sem subir no protótipo.
            if classe == Classe::ArrayTipado {
                match k {
                    ChaveProp::Indice(i) => return Ok(self.tipado_ler(o, i).unwrap_or(Valor::UNDEFINED)),
                    ChaveProp::Nome(c) if self.numerico_canonico(c) => return Ok(Valor::UNDEFINED),
                    _ => {}
                }
            }
            match k {
                ChaveProp::Nome(c) if !(c == fixos::LENGTH && matches!(self.formas.get(self.forma(o)).classe, Classe::Array | Classe::Caixa)) => {
                    if let Some(d) = self.propria(o, c) {
                        let v = self.ler_campo(o, d.slot);
                        if d.attrs & attr::ACESSOR != 0 {
                            let g = self.acessor_get(v);
                            return if g == Valor::UNDEFINED { Ok(Valor::UNDEFINED) } else { self.chamar(g, receptor, &[]) };
                        }
                        if v == Valor::BURACO && d.attrs & attr::PREGUICOSO != 0 {
                            return Ok(self.materializar_prototype(o));
                        }
                        return Ok(v);
                    }
                }
                _ => {
                    if let Some(d) = self.descritor_proprio(o, k) {
                        if d.acessor() {
                            return if d.get == Valor::UNDEFINED { Ok(Valor::UNDEFINED) } else { self.chamar(d.get, receptor, &[]) };
                        }
                        return Ok(d.valor);
                    }
                }
            }
            let p = self.proto_de(o);
            if p == Valor::NULL {
                return Ok(Valor::UNDEFINED);
            }
            o = p;
        }
    }

    /// Lê pelo nome (atalho para builtins).
    pub fn obter_nome(&mut self, alvo: Valor, c: Chave) -> Res<Valor> {
        self.obter(alvo, ChaveProp::Nome(c))
    }

    pub fn obter_str(&mut self, alvo: Valor, nome: &str) -> Res<Valor> {
        let c = self.nomes.internar_str(nome);
        self.obter(alvo, ChaveProp::Nome(c))
    }

    /// GetMethod: undefined/null viram `None`; não função é TypeError.
    pub fn obter_metodo(&mut self, alvo: Valor, k: ChaveProp) -> Res<Option<Valor>> {
        let f = self.obter(alvo, k)?;
        if f.e_nulo_ou_indefinido() {
            return Ok(None);
        }
        if !self.e_funcao(f) {
            let nome = self.chave_para_rust(k);
            return self.erro_tipo(&format!("{nome} não é uma função"));
        }
        Ok(Some(f))
    }

    /// `length` como número inteiro (LengthOfArrayLike).
    pub fn comprimento_de(&mut self, o: Valor) -> Res<u64> {
        if self.e_array(o) {
            return Ok(u64::from(self.array_len(o)));
        }
        let v = self.obter(o, ChaveProp::Nome(fixos::LENGTH))?;
        Ok(self.para_comprimento(v)? as u64)
    }

    /// Item `i` de um objeto (atalho com f64 para índices acima de u32).
    pub fn obter_indice(&mut self, o: Valor, i: u64) -> Res<Valor> {
        if i < u64::from(u32::MAX) {
            if let Some(v) = self.elemento_rapido(o, i as u32) {
                return Ok(v);
            }
            return self.obter(o, ChaveProp::Indice(i as u32));
        }
        let t = self.texto(&i.to_string());
        let k = self.chave_de_texto(t);
        self.obter(o, k)
    }

    /// Item próprio de array comum, sem passar pelos protótipos (None = caminho lento).
    #[inline]
    pub fn elemento_rapido(&self, o: Valor, i: u32) -> Option<Valor> {
        if !self.e_objeto(o) {
            return None;
        }
        self.elemento(o, i)
    }

    // ---------------------------------------------------------------- [[Set]]

    /// `alvo[k] = v`. `estrito`: falha vira TypeError (senão é ignorada).
    pub fn gravar(&mut self, alvo: Valor, k: ChaveProp, v: Valor, estrito: bool) -> Res<()> {
        if !self.e_objeto(alvo) {
            if alvo.e_nulo_ou_indefinido() || alvo == Valor::BURACO {
                let nome = self.chave_para_rust(k);
                let t = if alvo == Valor::NULL { "null" } else { "undefined" };
                return self.erro_tipo(&format!("Cannot set properties of {t} (setting '{nome}')"));
            }
            // Primitivo: só setter do protótipo tem efeito.
            let proto = self.proto_do_primitivo(alvo);
            if let Some(set) = self.setter_na_cadeia(proto, k) {
                self.chamar(set, alvo, &[v])?;
                return Ok(());
            }
            return if estrito { self.erro_tipo("Cannot create property on primitive") } else { Ok(()) };
        }
        let ok = self.gravar_com(alvo, k, v, alvo)?;
        if !ok && estrito {
            let nome = self.chave_para_rust(k);
            return self.erro_tipo(&format!("Cannot assign to read only property '{nome}' of object"));
        }
        Ok(())
    }

    fn setter_na_cadeia(&mut self, mut o: Valor, k: ChaveProp) -> Option<Valor> {
        loop {
            if let Some(d) = self.descritor_proprio(o, k) {
                return (d.acessor() && d.set != Valor::UNDEFINED).then_some(d.set);
            }
            o = self.proto_de(o);
            if o == Valor::NULL {
                return None;
            }
        }
    }

    /// OrdinarySet: devolve falso quando a gravação não é permitida.
    pub fn gravar_com(&mut self, o: Valor, k: ChaveProp, v: Valor, receptor: Valor) -> Res<bool> {
        let classe = self.formas.get(self.forma(o)).classe;
        if classe == Classe::Proxy {
            return self.proxy_gravar(o, k, v, receptor);
        }
        if classe == Classe::ArrayTipado {
            if let Some(i) = self.tipado_chave(o, k) {
                if receptor == o {
                    // Converte mesmo se o índice não vale (o valueOf roda).
                    self.tipado_gravar(o, i.unwrap_or(u32::MAX), v)?;
                    return Ok(true);
                }
                if !i.is_some_and(|i| i < self.tipado_len(o)) {
                    return Ok(true);
                }
            }
        }
        if receptor == o {
            match k {
                ChaveProp::Nome(c) if !(c == fixos::LENGTH && matches!(classe, Classe::Array | Classe::Caixa)) => {
                    // Caminho comum: dado próprio gravável.
                    if let Some(d) = self.propria(o, c) {
                        if d.attrs & (attr::ACESSOR | attr::GRAVAVEL) == attr::GRAVAVEL {
                            self.gravar_campo(o, d.slot, v);
                            return Ok(true);
                        }
                    }
                }
                ChaveProp::Indice(i) => {
                    if classe == Classe::ArrayTipado {
                        self.tipado_gravar(o, i, v)?;
                        return Ok(true);
                    }
                    if self.elemento(o, i).is_some() {
                        return Ok(self.gravar_elemento(o, i, v));
                    }
                }
                ChaveProp::Nome(_) => {
                    if classe == Classe::Array {
                        return self.gravar_length(o, v);
                    }
                }
            }
        }
        // Procura na cadeia: setter, só leitura, ou nada (cria no receptor).
        let mut p = o;
        loop {
            if self.formas.get(self.forma(p)).classe == Classe::Proxy && p != o {
                return self.proxy_gravar(p, k, v, receptor);
            }
            if p != o && self.formas.get(self.forma(p)).classe == Classe::ArrayTipado {
                if let Some(i) = self.tipado_chave(p, k) {
                    if p == receptor {
                        self.tipado_gravar(p, i.unwrap_or(u32::MAX), v)?;
                        return Ok(true);
                    }
                    if !i.is_some_and(|i| i < self.tipado_len(p)) {
                        return Ok(true);
                    }
                }
            }
            if let Some(d) = self.descritor_proprio(p, k) {
                if d.acessor() {
                    if d.set == Valor::UNDEFINED {
                        return Ok(false);
                    }
                    self.chamar(d.set, receptor, &[v])?;
                    return Ok(true);
                }
                if d.attrs & attr::GRAVAVEL == 0 {
                    return Ok(false);
                }
                break;
            }
            p = self.proto_de(p);
            if p == Valor::NULL {
                break;
            }
        }
        if !self.e_objeto(receptor) {
            return Ok(false);
        }
        // Grava (ou cria) a própria do receptor.
        if let Some(d) = self.descritor_proprio(receptor, k) {
            if d.acessor() || d.attrs & attr::GRAVAVEL == 0 {
                return Ok(false);
            }
        } else if !self.formas.get(self.forma(receptor)).extensivel {
            return Ok(false);
        }
        self.criar_dado(receptor, k, v)
    }

    fn gravar_length(&mut self, a: Valor, v: Valor) -> Res<bool> {
        let n = self.para_numero(v)?;
        let u = crate::numeros::para_uint32(n);
        if f64::from(u) != n {
            return self.erro_alcance("Invalid array length");
        }
        if self.propria(a, fixos::LENGTH).is_some() {
            // length marcado só leitura (freeze).
            return Ok(false);
        }
        self.definir_array_len(a, u);
        Ok(true)
    }

    /// CreateDataProperty: define dado próprio enumerável/gravável/configurável.
    pub fn criar_dado(&mut self, o: Valor, k: ChaveProp, v: Valor) -> Res<bool> {
        if self.formas.get(self.forma(o)).classe == Classe::Proxy {
            return self.proxy_criar_dado(o, k, v);
        }
        match k {
            ChaveProp::Indice(i) => {
                if self.indices_especiais {
                    let c = self.nome_do_indice(i);
                    if self.propria(o, c).is_some() {
                        self.definir_propria(o, c, v, attr::PADRAO);
                        return Ok(true);
                    }
                }
                Ok(self.gravar_elemento(o, i, v))
            }
            ChaveProp::Nome(fixos::LENGTH) if self.e_array(o) => self.gravar_length(o, v),
            ChaveProp::Nome(c) => {
                match self.propria(o, c) {
                    Some(d) if d.attrs == attr::PADRAO => {
                        self.gravar_campo(o, d.slot, v);
                    }
                    Some(d) if d.attrs & attr::CONFIGURAVEL == 0 => return Ok(false),
                    _ => {
                        if self.propria(o, c).is_none() && !self.formas.get(self.forma(o)).extensivel {
                            return Ok(false);
                        }
                        self.definir_propria(o, c, v, attr::PADRAO);
                    }
                }
                Ok(true)
            }
        }
    }

    /// Define dado com atributos (para builtins; sem validação).
    pub fn definir_dado(&mut self, o: Valor, k: ChaveProp, v: Valor, attrs: u8) {
        match k {
            ChaveProp::Indice(i) if attrs == attr::PADRAO => {
                self.gravar_elemento(o, i, v);
            }
            ChaveProp::Indice(i) => {
                self.apagar_elemento(o, i);
                self.indices_especiais = true;
                let c = self.nome_do_indice(i);
                self.definir_propria(o, c, v, attrs);
            }
            ChaveProp::Nome(c) => self.definir_propria(o, c, v, attrs),
        }
    }

    // ---------------------------------------------------------------- defineProperty

    /// `[[DefineOwnProperty]]` com validação (§10.1.6.3). Devolve falso se não pode.
    pub fn definir_propriedade(&mut self, mut o: Valor, k: ChaveProp, d: DescParcial) -> Res<bool> {
        // shortcut: Proxy sem armadilha defineProperty: define no alvo.
        while self.formas.get(self.forma(o)).classe == Classe::Proxy {
            o = Valor(self.interno(o, 0));
        }
        if let ChaveProp::Nome(c) = k {
            if self.formas.get(self.forma(o)).classe == Classe::ArrayTipado && self.numerico_canonico(c) {
                return Ok(false);
            }
        }
        if let ChaveProp::Indice(i) = k {
            if self.formas.get(self.forma(o)).classe == Classe::ArrayTipado {
                // Item de array tipado: só dado gravável, enumerável e configurável, dentro do comprimento.
                if i >= self.tipado_len(o) || d.e_acessor() || d.configuravel == Some(false) || d.enumeravel == Some(false) || d.gravavel == Some(false) {
                    return Ok(false);
                }
                if let Some(v) = d.valor {
                    self.tipado_gravar(o, i, v)?;
                }
                return Ok(true);
            }
        }
        if self.e_array(o) && k == ChaveProp::Nome(fixos::LENGTH) {
            if let Some(v) = d.valor {
                if !self.gravar_length(o, v)? {
                    return Ok(false);
                }
            }
            if d.gravavel == Some(false) {
                let n = self.array_len(o);
                self.definir_propria(o, fixos::LENGTH, Valor::int(n as i32), 0);
            }
            return Ok(true);
        }
        let atual = self.descritor_proprio(o, k);
        let Some(atual) = atual else {
            if !self.formas.get(self.forma(o)).extensivel {
                return Ok(false);
            }
            let mut a = 0;
            if d.enumeravel == Some(true) {
                a |= attr::ENUMERAVEL;
            }
            if d.configuravel == Some(true) {
                a |= attr::CONFIGURAVEL;
            }
            if d.e_acessor() {
                let par = self.novo_acessor(d.get.unwrap_or(Valor::UNDEFINED), d.set.unwrap_or(Valor::UNDEFINED));
                self.definir_bruto(o, k, par, a | attr::ACESSOR);
            } else {
                if d.gravavel == Some(true) {
                    a |= attr::GRAVAVEL;
                }
                self.definir_dado(o, k, d.valor.unwrap_or(Valor::UNDEFINED), a);
            }
            return Ok(true);
        };
        let configuravel = atual.attrs & attr::CONFIGURAVEL != 0;
        if !configuravel {
            if d.configuravel == Some(true) {
                return Ok(false);
            }
            if let Some(e) = d.enumeravel {
                if e != (atual.attrs & attr::ENUMERAVEL != 0) {
                    return Ok(false);
                }
            }
            if d.e_acessor() != atual.acessor() && (d.e_acessor() || d.valor.is_some() || d.gravavel.is_some()) {
                return Ok(false);
            }
            if atual.acessor() {
                let mesmo_get = d.get.map_or(true, |g| g == atual.get);
                let mesmo_set = d.set.map_or(true, |s| s == atual.set);
                if !(mesmo_get && mesmo_set) {
                    return Ok(false);
                }
                return Ok(true);
            }
            if atual.attrs & attr::GRAVAVEL == 0 {
                if d.gravavel == Some(true) {
                    return Ok(false);
                }
                if let Some(v) = d.valor {
                    if !self.mesmo_valor(v, atual.valor) {
                        return Ok(false);
                    }
                }
                return Ok(true);
            }
        }
        // Junta o atual com o novo.
        let enumeravel = d.enumeravel.unwrap_or(atual.attrs & attr::ENUMERAVEL != 0);
        let configuravel = d.configuravel.unwrap_or(configuravel);
        let mut a = 0;
        if enumeravel {
            a |= attr::ENUMERAVEL;
        }
        if configuravel {
            a |= attr::CONFIGURAVEL;
        }
        let vira_acessor = d.e_acessor() || (atual.acessor() && d.valor.is_none() && d.gravavel.is_none());
        if vira_acessor {
            let (g0, s0) = if atual.acessor() { (atual.get, atual.set) } else { (Valor::UNDEFINED, Valor::UNDEFINED) };
            let par = self.novo_acessor(d.get.unwrap_or(g0), d.set.unwrap_or(s0));
            self.definir_bruto(o, k, par, a | attr::ACESSOR);
        } else {
            let gravavel = d.gravavel.unwrap_or(!atual.acessor() && atual.attrs & attr::GRAVAVEL != 0);
            if gravavel {
                a |= attr::GRAVAVEL;
            }
            let v = d.valor.unwrap_or(if atual.acessor() { Valor::UNDEFINED } else { atual.valor });
            if let ChaveProp::Indice(i) = k {
                if a == attr::PADRAO && self.elemento(o, i).is_some() {
                    self.gravar_elemento(o, i, v);
                    return Ok(true);
                }
            }
            self.definir_dado(o, k, v, a);
        }
        Ok(true)
    }

    /// Grava o campo cru (par de acessores) com os atributos.
    fn definir_bruto(&mut self, o: Valor, k: ChaveProp, v: Valor, attrs: u8) {
        let c = match k {
            ChaveProp::Indice(i) => {
                self.apagar_elemento(o, i);
                self.indices_especiais = true;
                self.nome_do_indice(i)
            }
            ChaveProp::Nome(c) => c,
        };
        self.definir_propria(o, c, v, attrs);
    }

    /// Define getter/setter (literal de objeto e classe): junta com o par que já existe.
    pub fn definir_acessor(&mut self, o: Valor, k: ChaveProp, f: Valor, setter: bool, enumeravel: bool) {
        let atual = self.descritor_proprio(o, k).filter(|d| d.acessor());
        let (mut g, mut s) = atual.map_or((Valor::UNDEFINED, Valor::UNDEFINED), |d| (d.get, d.set));
        if setter {
            s = f;
        } else {
            g = f;
        }
        let par = self.novo_acessor(g, s);
        let a = attr::CONFIGURAVEL | attr::ACESSOR | if enumeravel { attr::ENUMERAVEL } else { 0 };
        self.definir_bruto(o, k, par, a);
    }

    // ---------------------------------------------------------------- has / delete

    /// `k in o`.
    pub fn tem(&mut self, mut o: Valor, k: ChaveProp) -> Res<bool> {
        loop {
            if self.formas.get(self.forma(o)).classe == Classe::Proxy {
                return self.proxy_tem(o, k);
            }
            if self.tem_propria(o, k) {
                return Ok(true);
            }
            // Índice de TypedArray fora do tamanho: não existe, e não se procura no protótipo.
            if self.formas.get(self.forma(o)).classe == Classe::ArrayTipado && self.tipado_chave(o, k).is_some() {
                return Ok(false);
            }
            o = self.proto_de(o);
            if o == Valor::NULL {
                return Ok(false);
            }
        }
    }

    pub fn tem_propria(&mut self, o: Valor, k: ChaveProp) -> bool {
        match k {
            ChaveProp::Nome(c) if c != fixos::LENGTH => self.propria(o, c).is_some(),
            _ => self.descritor_proprio(o, k).is_some(),
        }
    }

    /// `delete o[k]`; falso se não é configurável.
    pub fn apagar(&mut self, mut o: Valor, k: ChaveProp) -> Res<bool> {
        while self.formas.get(self.forma(o)).classe == Classe::Proxy {
            o = Valor(self.interno(o, 0));
        }
        Ok(match k {
            ChaveProp::Indice(i) if self.formas.get(self.forma(o)).classe == Classe::ArrayTipado => i >= self.tipado_len(o),
            ChaveProp::Indice(i) => {
                if self.elemento(o, i).is_some() {
                    self.apagar_elemento(o, i)
                } else if let Some(t) = self.texto_da_caixa(o) {
                    i >= self.texto_len(t)
                } else if self.indices_especiais {
                    let c = self.nome_do_indice(i);
                    self.apagar_propria(o, c)
                } else {
                    true
                }
            }
            ChaveProp::Nome(fixos::LENGTH) if self.e_array(o) => false,
            ChaveProp::Nome(c) => self.apagar_propria(o, c),
        })
    }

    // ---------------------------------------------------------------- chaves

    /// `[[OwnPropertyKeys]]`: índices em ordem, depois nomes, depois símbolos (na ordem de
    /// criação). `enumeraveis`: só as enumeráveis; `simbolos`: inclui símbolos; `nomes`: inclui textos.
    pub fn chaves_de(&mut self, mut o: Valor, enumeraveis: bool, nomes: bool, simbolos: bool) -> Vec<ChaveProp> {
        let mut r = Vec::new();
        while self.formas.get(self.forma(o)).classe == Classe::Proxy {
            o = Valor(self.interno(o, 0));
        }
        if nomes {
            if self.formas.get(self.forma(o)).classe == Classe::ArrayTipado {
                r.extend((0..self.tipado_len(o)).map(ChaveProp::Indice));
            }
            if let Some(t) = self.texto_da_caixa(o) {
                for i in 0..self.texto_len(t) {
                    r.push(ChaveProp::Indice(i));
                }
            }
            let el = self.elementos(o);
            if el != Valor::LISTA_VAZIA {
                let n = self.lista_len(el);
                let n = if self.e_array(o) { n.min(self.array_len(o)) } else { n };
                for i in 0..n {
                    if self.lista_get(el, i) != Valor::BURACO {
                        r.push(ChaveProp::Indice(i));
                    }
                }
            }
            if self.e_array(o) && !enumeraveis {
                r.push(ChaveProp::Nome(fixos::LENGTH));
            }
        }
        let descr = self.chaves_proprias(o);
        let mut especiais: Vec<(u32, ChaveProp)> = Vec::new();
        let mut textos = Vec::new();
        let mut simbs = Vec::new();
        for d in descr {
            if enumeraveis && d.attrs & attr::ENUMERAVEL == 0 {
                continue;
            }
            if d.chave.e_simbolo() {
                let id = d.chave.0 & !Chave::BIT_SIMBOLO;
                if simbolos && !self.escondidos.contains(&id) && d.chave != simb::CAMPOS {
                    simbs.push(ChaveProp::Nome(d.chave));
                }
            } else if nomes {
                if d.chave == fixos::LENGTH && self.e_array(o) {
                    continue;
                }
                if self.indices_especiais {
                    let u = self.nomes.texto(d.chave).to_vec();
                    if let Some(i) = crate::textos::indice_de_array(&u) {
                        especiais.push((i, ChaveProp::Indice(i)));
                        continue;
                    }
                }
                textos.push(ChaveProp::Nome(d.chave));
            }
        }
        if !especiais.is_empty() {
            especiais.sort_by_key(|e| e.0);
            r.extend(especiais.into_iter().map(|e| e.1));
            r.sort_by_key(|k| match k {
                ChaveProp::Indice(i) => *i,
                _ => u32::MAX,
            });
        }
        r.extend(textos);
        r.extend(simbs);
        r
    }

    // ---------------------------------------------------------------- Proxy

    fn armadilha(&mut self, p: Valor, nome: &str) -> Res<Option<(Valor, Valor)>> {
        let tratador = Valor(self.interno(p, 1));
        if tratador == Valor::NULL {
            return self.erro_tipo("Cannot perform operation on a revoked proxy");
        }
        let c = self.nomes.internar_str(nome);
        let f = self.obter_metodo(tratador, ChaveProp::Nome(c))?;
        Ok(f.map(|f| (f, tratador)))
    }

    fn proxy_obter(&mut self, p: Valor, k: ChaveProp, receptor: Valor) -> Res<Valor> {
        self.checar_pilha()?;
        let alvo = Valor(self.interno(p, 0));
        match self.armadilha(p, "get")? {
            Some((f, t)) => {
                let kv = self.chave_para_valor(k);
                let v = self.chamar(f, t, &[alvo, kv, receptor])?;
                // Invariantes (§10.5.8): a armadilha não pode mentir sobre o que o alvo fixou.
                if let Some(d) = self.descritor_proprio(alvo, k).filter(|d| d.attrs & attr::CONFIGURAVEL == 0) {
                    let fixo = !d.acessor() && d.attrs & attr::GRAVAVEL == 0;
                    if (fixo && !self.mesmo_valor(v, d.valor)) || (d.acessor() && d.get == Valor::UNDEFINED && v != Valor::UNDEFINED) {
                        return self.erro_tipo("'get' on proxy: result differs from the non-configurable target property");
                    }
                }
                Ok(v)
            }
            None => self.obter_com(alvo, k, receptor),
        }
    }

    fn proxy_gravar(&mut self, p: Valor, k: ChaveProp, v: Valor, receptor: Valor) -> Res<bool> {
        self.checar_pilha()?;
        let alvo = Valor(self.interno(p, 0));
        match self.armadilha(p, "set")? {
            Some((f, t)) => {
                let kv = self.chave_para_valor(k);
                let r = self.chamar(f, t, &[alvo, kv, v, receptor])?;
                if !self.verdadeiro(r) {
                    return Ok(false);
                }
                if let Some(d) = self.descritor_proprio(alvo, k).filter(|d| d.attrs & attr::CONFIGURAVEL == 0) {
                    let fixo = !d.acessor() && d.attrs & attr::GRAVAVEL == 0;
                    if (fixo && !self.mesmo_valor(v, d.valor)) || (d.acessor() && d.set == Valor::UNDEFINED) {
                        return self.erro_tipo("'set' on proxy: trap returned true for a non-writable target property");
                    }
                }
                Ok(true)
            }
            None => {
                let receptor = if receptor == p { alvo } else { receptor };
                self.gravar_com(alvo, k, v, receptor)
            }
        }
    }

    /// CreateDataProperty num Proxy: a armadilha `defineProperty` com o descritor de dado padrão.
    /// shortcut: sem as invariantes de §10.5.6 (alvo não configurável/não extensível); entra com
    /// o resto das armadilhas de descritor.
    fn proxy_criar_dado(&mut self, p: Valor, k: ChaveProp, v: Valor) -> Res<bool> {
        self.checar_pilha()?;
        let alvo = Valor(self.interno(p, 0));
        match self.armadilha(p, "defineProperty")? {
            Some((f, t)) => {
                let kv = self.chave_para_valor(k);
                let d = self.objeto_vazio();
                self.definir_propria(d, fixos::VALUE, v, attr::PADRAO);
                self.definir_propria(d, fixos::WRITABLE, Valor::TRUE, attr::PADRAO);
                self.definir_propria(d, fixos::ENUMERABLE, Valor::TRUE, attr::PADRAO);
                self.definir_propria(d, fixos::CONFIGURABLE, Valor::TRUE, attr::PADRAO);
                let r = self.chamar(f, t, &[alvo, kv, d])?;
                Ok(self.verdadeiro(r))
            }
            None => self.criar_dado(alvo, k, v),
        }
    }

    fn proxy_tem(&mut self, p: Valor, k: ChaveProp) -> Res<bool> {
        self.checar_pilha()?;
        let alvo = Valor(self.interno(p, 0));
        match self.armadilha(p, "has")? {
            Some((f, t)) => {
                let kv = self.chave_para_valor(k);
                let r = self.chamar(f, t, &[alvo, kv])?;
                if self.verdadeiro(r) {
                    return Ok(true);
                }
                if let Some(d) = self.descritor_proprio(alvo, k) {
                    if d.attrs & attr::CONFIGURAVEL == 0 || !self.formas.get(self.forma(alvo)).extensivel {
                        return self.erro_tipo("'has' on proxy: trap returned false for a property the target fixes");
                    }
                }
                Ok(false)
            }
            None => self.tem(alvo, k),
        }
    }
}
