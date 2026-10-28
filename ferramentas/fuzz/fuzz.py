#!/usr/bin/env python3
"""Fuzzing diferencial do Falcão: gera JS válido aleatório (voltado às classes de bug de motor),
roda no Falcão e numa referência (shell do SpiderMonkey) e separa quedas, travamentos, falta de
memória e saídas diferentes. Cada achado é reduzido (delta debugging) e deduplicado.

uso:
  python3 -I fuzz.py --falcao BIN --ref JS_SHELL --saida DIR [--minutos 90] [--max 100000]
                     [--trabalhadores 6] [--semente N] [--duplo]
  --duplo: compara o Falcão com ele mesmo com FALCAO_JIT=tudo (N1 contra N2), sem a referência.
  python3 -I fuzz.py --teste: confere o gerador e o redutor.

Só biblioteca padrão: o gerador é texto, o resto é subprocesso.
"""
import argparse, concurrent.futures as cf, hashlib, json, os, random, re, resource, subprocess, sys, tempfile, threading, time

# ---------------------------------------------------------------- prelúdio (igual nos dois motores)

# Tudo capturado no início: os blocos mexem em protótipos, e a saída não pode depender disso.
PRELUDIO = r"""
var __P = print, __is = Object.is, __keys = Object.keys, __isArr = Array.isArray, __apply = Reflect.apply;
var __toStr = Object.prototype.toString, __slice = String.prototype.slice, __isView = ArrayBuffer.isView;
function nm(e) {
  try {
    if (e === null || e === undefined) return 'lancou:' + e;
    if (typeof e === 'object' || typeof e === 'function') {
      var n = e.name;
      if (n === 'InternalError') n = 'RangeError';
      return 'E:' + (typeof n === 'string' ? n : '?');
    }
    return 'lancou:' + typeof e + ':' + S(e, 0);
  } catch (x) { return 'E:?'; }
}
function S(v, d) {
  try {
    if (d > 3) return '..';
    var t = typeof v;
    if (t === 'number') return __is(v, -0) ? '-0' : '' + v;
    if (t === 'string') return v.length > 60 ? 's' + v.length + ':' + __apply(__slice, v, [0, 40]) : JSON.stringify(v);
    if (t === 'bigint') return v + 'n';
    if (t === 'symbol') return 'sym';
    if (t === 'function') return 'fn';
    if (t !== 'object' || v === null) return '' + v;
    var tag = __apply(__toStr, v, []);
    if (__isArr(v) || (__isView(v) && tag !== '[object DataView]')) {
      var n = v.length, r = tag === '[object Array]' ? '[' : tag + '[', i;
      for (i = 0; i < n && i < 16; i++) r += (i ? ',' : '') + ((i in v) ? S(v[i], d + 1) : '_');
      return r + (n > 16 ? ',..' + n : '') + ']';
    }
    if (tag !== '[object Object]') return tag;
    var ks = __keys(v), r = '{', j;
    for (j = 0; j < ks.length && j < 12; j++) r += (j ? ',' : '') + ks[j] + ':' + S(v[ks[j]], d + 1);
    return r + (ks.length > 12 ? ',..' : '') + '}';
  } catch (e) { return 'S!' + nm(e); }
}
function out(tag, v) { var s = S(v, 0); if (s.length > 300) s = __apply(__slice, s, [0, 300]) + '..'; __P(tag + ' ' + s); }
"""

# ---------------------------------------------------------------- gerador

INTS = ['0', '1', '-1', '2', '3', '7', '-0', '0.5', 'NaN', 'Infinity', '-Infinity', '2147483647', '2147483648',
        '-2147483648', '-2147483649', '4294967295', '4294967296', '9007199254740991', '9007199254740992',
        '-9007199254740991', '1e21', '1.5', '-7']
PEQ = ['0', '1', '2', '3', '-1', '-2', '5', '0.5', 'NaN', 'undefined', "'2'", 'null', 'true']
VALS = ['0', '1', '-0', 'NaN', "'a'", "''", "'10'", 'null', 'undefined', 'true', '[]', '{}', '[1,2]', '1n',
        'Symbol.iterator', '{valueOf(){return 3}}', '{toString(){return "x"}}', '-1', '2147483648']
METODOS_ARRAY = [
    ('indexOf', '{v}, {p}'), ('lastIndexOf', '{v}, {p}'), ('includes', '{v}, {p}'), ('join', '{s}'),
    ('reverse', ''), ('slice', '{p}, {p}'), ('splice', '{p}, {p}, {v}'), ('concat', '{arr}'),
    ('fill', '{v}, {p}, {p}'), ('copyWithin', '{p}, {p}, {p}'), ('flat', '{p}'), ('push', '{v}, {v}'),
    ('pop', ''), ('shift', ''), ('unshift', '{v}'), ('at', '{p}'), ('findLast', 'x => x === {v}'),
    ('toReversed', ''), ('toSpliced', '{p}, {p}, {v}'), ('with', '{p}, {v}'), ('keys', ''), ('entries', ''),
    ('every', 'x => x !== {v}'), ('some', 'x => x === {v}'), ('filter', 'x => x !== {v}'),
    ('map', 'x => typeof x'), ('reduce', '(a, b) => a + "" + b, ""'), ('reduceRight', '(a, b) => a + "" + b, ""'),
    ('find', 'x => x === {v}'), ('findIndex', 'x => x === {v}'), ('flatMap', 'x => [x, x]'), ('toString', ''),
]
TIPADOS = ['Int8Array', 'Uint8Array', 'Uint8ClampedArray', 'Int16Array', 'Uint16Array', 'Int32Array', 'Uint32Array',
           'Float32Array', 'Float64Array', 'BigInt64Array', 'BigUint64Array']


class Gerador:
    def __init__(self, semente):
        self.r = random.Random(semente)

    def e(self, l):
        return self.r.choice(l)

    def arr(self):
        r = self.r
        k = r.randint(0, 6)
        itens = []
        for _ in range(k):
            itens.append('' if r.random() < 0.15 else self.e(VALS + PEQ))
        s = '[' + ','.join(itens) + (',' if itens and itens[-1] == '' else '') + ']'
        return s

    def p(self):
        return self.e(PEQ + INTS[:8])

    def preencher(self, molde):
        out = molde
        for marca, f in (('{v}', lambda: self.e(VALS)), ('{p}', self.p), ('{s}', lambda: self.e(["','", "''", "undefined", "'--'", '1'])),
                         ('{arr}', self.arr), ('{n}', lambda: self.e(INTS)), ('{i}', lambda: self.e(PEQ))):
            while marca in out:
                out = out.replace(marca, f(), 1)
        return out

    # Cada bloco: (tipo, código). O código usa `out('#k ...', valor)`.
    def bloco_array(self):
        a = self.arr()
        m, args = self.e(METODOS_ARRAY)
        extra = ''
        if self.r.random() < 0.3:
            extra = 'a.length = %s; ' % self.e(['0', '1', '3', '10', '100', "'4'"])
        if self.r.random() < 0.15:
            extra += 'delete a[%s]; ' % self.e(['0', '1', '2'])
        r = 'var a = %s; %svar r = a.%s(%s); out("#k", r); out("#k", a);' % (a, extra, m, self.preencher(args))
        if m in ('keys', 'entries'):
            r = 'var a = %s; %sout("#k", [...a.%s()]);' % (a, extra, m)
        return 'array', r

    def bloco_mutacao(self):
        mut = self.e(['a.length = 0', 'a.length = 1', 'a.push(9, 9)', 'a.pop()', 'a.shift()', 'a.unshift(7)',
                      'a[5] = 5', 'delete a[1]', 'a.length = 10', 'a.reverse()', 'a.fill(4)', 'a.splice(0, 1)'])
        a = self.e(['[1,2,3,4]', '[1,,3,,5]', '["a","b","c"]', '[1.5,2.5]', '[{},[],null]'])
        if self.r.random() < 0.5:
            m = self.e(['forEach(f)', 'map(f)', 'filter(f)', 'some(f)', 'every(f)', 'reduce(f, 0)', 'find(f)', 'findLastIndex(f)',
                        'flatMap(f)', 'sort((x, y) => (f(x, 0), 0))'])
            corpo = 'var a = %s, c = 0; var f = function (x, i) { if (c++ === %d) { %s; } return i %% 2 === 0; }; var r = a.%s; out("#k", r); out("#k", a);' % (
                a, self.r.randint(0, 3), mut, m)
        else:
            m = self.e(['slice(o, o)', 'splice(o, o)', 'fill(0, o, o)', 'copyWithin(o, o)', 'indexOf(1, o)', 'lastIndexOf(1, o)',
                        'includes(1, o)', 'at(o)', 'with(o, o)', 'join(o)', 'toSpliced(o, o)', 'flat(o)'])
            corpo = 'var a = %s; var o = { valueOf() { %s; return %s; }, toString() { return ","; } }; var r = a.%s; out("#k", r); out("#k", a);' % (
                a, mut, self.e(['1', '0', '2', '-1']), m)
        return 'mutacao', corpo

    def bloco_sort(self):
        a = self.e(['[3,1,2]', '[5,,1,undefined,2]', '["b","a","c","aa"]', '[3,1,2,1e21,-0,0,NaN]', '[{},[],1,"1"]'])
        cmp = self.e(['', '(x, y) => x - y', '(x, y) => y - x', '(x, y) => (x > y) - (x < y)'])
        mut = self.r.random() < 0.4
        if mut:
            corpo = 'var a = %s; a.sort((x, y) => { %s; return 0; }); out("#k", a.length >= 0);' % (
                a, self.e(['a.length = 0', 'a.push(1)', 'a[0] = 9', 'a.pop()', 'delete a[0]']))
        else:
            corpo = 'var a = %s; a.sort(%s); out("#k", a); out("#k", a.toSorted(%s));' % (a, cmp, cmp)
        return 'sort', corpo

    def bloco_getter_proto(self):
        i = self.e(['0', '1', '2', '3'])
        proto = self.e(['Array', 'Object'])
        acao = self.e(['return 7', 'a.length = 0; return 1', 'a.push(1); return 2', 'throw new TypeError("g")', 'return undefined'])
        m, args = self.e(METODOS_ARRAY)
        corpo = ('var a = [1,,3,,5]; var g = 0; Object.defineProperty(%s.prototype, "%s", { get() { if (g++ > 20) return 0; %s; }, configurable: true }); '
                 'try { var r = a.%s(%s); out("#k", r); out("#k", a); } finally { delete %s.prototype["%s"]; }') % (
                     proto, i, acao, m, self.preencher(args), proto, i)
        return 'getter_proto', corpo

    def bloco_especie(self):
        m = self.e(['map(x => x)', 'filter(x => true)', 'slice(0)', 'splice(0, 1)', 'concat([9])', 'flat()', 'flatMap(x => x)'])
        c = self.e(['function (n) { return {length: 0}; }', 'function (n) { return [9, 9, 9]; }', 'function (n) { return Object.freeze([]); }',
                    'function (n) { this.x = n; }', 'undefined', 'null', 'Array', '42'])
        corpo = 'var a = [1,2,3]; a.constructor = {[Symbol.species]: %s}; var r = a.%s; out("#k", r);' % (c, m)
        return 'especie', corpo

    def bloco_proxy(self):
        traps = self.r.sample([
            'get(t, k, r) { return k === "length" ? %s : Reflect.get(t, k, r); }' % self.e(['3', '0', '"2"', '-1', '4294967296']),
            'has(t, k) { return %s; }' % self.e(['true', 'false', 'k in t']),
            'set(t, k, v, r) { return %s; }' % self.e(['Reflect.set(t, k, v, r)', 'false', 'true']),
            'deleteProperty(t, k) { return %s; }' % self.e(['false', 'true', 'delete t[k]']),
            'getPrototypeOf(t) { return %s; }' % self.e(['null', 'Array.prototype', 'Object.prototype']),
        ], self.r.randint(1, 3))
        alvo = self.e(['[1,2,3]', '{0: 1, 1: 2, length: 2}', 'Object.freeze([1])', '{}'])
        m, args = self.e(METODOS_ARRAY)
        uso = self.e(['Array.prototype.%s.call(p, %s)' % (m, self.preencher(args)), 'Object.keys(p)', '"0" in p', 'p.length',
                      'delete p[0]', 'Array.isArray(p)', '[...Array.prototype.values.call(p)]', 'JSON.stringify(p)'])
        corpo = 'var p = new Proxy(%s, { %s }); out("#k", %s);' % (alvo, ', '.join(traps), uso)
        return 'proxy', corpo

    def bloco_tipado(self):
        t = self.e(TIPADOS)
        big = t.startswith('Big')
        val = self.e(['1n', '-1n', '2n ** 63n', '0n']) if big else self.e(INTS + ['"3"', 'undefined'])
        rz = self.r.random() < 0.4
        criar = 'var b = new ArrayBuffer(16%s); var ta = new %s(b%s);' % (
            ', {maxByteLength: 64}' if rz else '', t, self.e(['', ', 8', ', 0, 1']) if t in ('Int8Array', 'Uint8Array', 'Uint8ClampedArray') else self.e(['', ', 8']))
        op = self.e([
            'ta[0] = %s; out("#k", ta[0]);' % val,
            'ta.fill(%s); out("#k", ta);' % val,
            'out("#k", ta.subarray(%s, %s));' % (self.p(), self.p()),
            'out("#k", ta.slice(%s));' % self.p(),
            'ta.set([%s], %s); out("#k", ta);' % (val, self.e(['0', '1'])),
            'out("#k", ta.map(x => x));',
            'out("#k", ta.indexOf(%s));' % val,
            'out("#k", [ta.length, ta.byteLength, ta.byteOffset]);',
            'ta.sort(); out("#k", ta);',
            'out("#k", ta.toReversed());',
            'out("#k", ta.with(%s, %s));' % (self.e(['0', '1', '-1', '100']), val),
            'out("#k", Array.from(ta));',
        ])
        mexe = self.e([
            '',
            'detachArrayBuffer(b); ',
            'detachArrayBuffer(b); ' if not rz else 'b.resize(%s); ' % self.e(['0', '4', '8', '32', '64']),
            'var vo = {valueOf() { detachArrayBuffer(b); return 1; }}; ',
            ('var vo = {valueOf() { b.resize(0); return 1; }}; ' if rz else ''),
        ])
        if mexe.startswith('var vo'):
            op = self.e(['ta.fill(1, vo); out("#k", ta);', 'out("#k", ta.subarray(vo));', 'out("#k", ta.slice(vo));',
                         'ta.copyWithin(0, vo); out("#k", ta);', 'out("#k", ta.indexOf(0, vo));', 'out("#k", ta.at(vo));',
                         'ta[0] = vo; out("#k", ta[0]);' if not big else 'out("#k", ta.length);'])
        return 'tipado', criar + mexe + op

    def bloco_inteiros(self):
        a, b = self.e(INTS), self.e(INTS)
        op = self.e(['+', '-', '*', '/', '%', '|', '&', '^', '<<', '>>', '>>>', '**'])
        x = self.e([
            'out("#k", (%s) %s (%s));' % (a, op, b),
            'out("#k", ((%s) %s (%s)) | 0);' % (a, op, b),
            'var x = %s; x++; x--; x += %s; out("#k", [x, x | 0, x >>> 0, ~x, -x]);' % (a, b),
            'out("#k", [Math.floor(%s), Math.round(%s), Math.trunc(%s), Math.sign(%s), Math.abs(%s)]);' % (a, a, a, a, a),
            'out("#k", (%s).toString(%s));' % (a, self.e(['2', '16', '36', '10'])),
            'out("#k", [(%s).toFixed(%s), (%s).toPrecision(%s)]);' % (a, self.e(['0', '2', '20']), a, self.e(['1', '5', '21'])),
            'var s = 0; for (var i = %s; i < %s + 5; i++) { s = (s + i) | 0; } out("#k", s);' % (a, a),
            'out("#k", [Number.isInteger(%s), Number.isSafeInteger(%s), Object.is(%s, -0)]);' % (a, a, a),
            'out("#k", BigInt.asIntN(%s, %sn));' % (self.e(['8', '32', '64', '0']), self.e(['255', '-1', '2147483648', '18446744073709551615'])),
        ])
        return 'inteiros', x

    def bloco_textos(self):
        s = self.e(['""', '"abc"', '"a".repeat(%s)' % self.e(['0', '1', '1000', '100000']), '"\\u00e9\\u4e2d\\ud83d\\ude00"', '"\\ud800x"', '"a,b,,c"'])
        op = self.e(['padStart(%s, "xy")' % self.e(['0', '5', '1000']), 'slice(%s, %s)' % (self.p(), self.p()), 'split(%s)' % self.e(['","', '""', 'undefined']),
                     'at(%s)' % self.p(), 'codePointAt(%s)' % self.p(), 'replaceAll("a", "$&$&")', 'indexOf("b", %s)' % self.p(),
                     'repeat(%s)' % self.e(['0', '2', '-1', 'Infinity']), 'toUpperCase()', 'trim()', 'isWellFormed()', 'toWellFormed()',
                     'concat(1, null)', 'substring(%s, %s)' % (self.p(), self.p()), 'normalize("NFC")', 'charCodeAt(%s)' % self.p()])
        if op.startswith('normalize') and 'ud800' in s:
            s = '"abc"'  # normalize de substituto solto: o Firefox troca por U+FFFD, a norma não fixa
        return 'textos', 'out("#k", (%s).%s);' % (s, op)

    def bloco_regex(self):
        re_ = self.e([r'/(a+)+b/', r'/(\d+)-(?<m>\d+)/g', r'/(?<=\$)\d+/', r'/a*?/g', r'/^.$/u', r'/[\p{L}]+/gu', r'/(a|ab)(c|bcd)(d*)/',
                      r'/x/y', r'/\bfoo\b/i', r'/(?:)/g', r'/(.)\1/', r'/[^a-c]/g', r'/\s+/g', r'/\u{1F600}/u', r'/[[a-z]--[aeiou]]/v'])
        s = self.e(['"aaaaaaaaaaaaaaab"', '"2024-05 10-11"', '"$12 $3"', '"abcd"', '"\\ud83d\\ude00"', '"foo Foo bar"', '"aa bb"', '""', '"xyz"'])
        op = self.e(['out("#k", %s.exec(%s));' % (re_, s), 'out("#k", %s.match(%s));' % (s, re_), 'out("#k", %s.replace(%s, "[$1]"));' % (s, re_),
                     'out("#k", %s.replace(%s, (m, ...r) => m.length));' % (s, re_), 'out("#k", %s.split(%s));' % (s, re_),
                     'out("#k", [...%s.matchAll(%s)].length);' % (s, re_ if 'g' in re_.split('/')[-1] else re_ + 'g'),
                     'var r = %s; r.lastIndex = %s; out("#k", [r.test(%s), r.lastIndex]);' % (re_, self.p(), s),
                     'out("#k", %s.search(%s));' % (s, re_)])
        return 'regex', op

    def bloco_json(self):
        txt = self.e(["'[1,{\"a\":[2,3]},\"x\"]'", "'{\"a\":1,\"b\":{\"c\":2},\"a\":3}'", "'[-0, 1e400, 0.1]'", "'{\"__proto__\":1}'", "'[[[[]]]]'"])
        rev = self.e(['undefined', '(k, v) => typeof v === "number" ? v + 1 : v', 'function (k, v) { if (k === "0") delete this[1]; return v; }',
                      'function (k, v) { if (Array.isArray(this)) this.length = 0; return v; }', '(k, v) => undefined'])
        st = self.e(['undefined', '["a"]', '(k, v) => v', 'function (k, v) { return k === "b" ? undefined : v; }'])
        x = self.e(['out("#k", JSON.parse(%s, %s));' % (txt, rev),
                    'out("#k", JSON.stringify(JSON.parse(%s), %s, %s));' % (txt, st, self.e(['undefined', '2', '"--"', '20'])),
                    'out("#k", JSON.stringify({a: 1, toJSON() { return [this.a, 2]; }, b: [undefined, function () {}, Symbol()]}));',
                    'out("#k", JSON.stringify([new Number(3), new String("s"), new Boolean(false), -0, 1e21, NaN]));'])
        return 'json', x

    def bloco_classe(self):
        x = self.e([
            'class A { #x = %s; static #c = 0; get x() { return this.#x; } static has(o) { return #x in o; } m() { return ++this.#x; } } var a = new A(); a.m(); out("#k", [a.x, A.has(a), A.has({})]);' % self.p(),
            'class A { #p() { return 1; } static t(o) { return o.#p(); } } try { out("#k", A.t(new A())); A.t({}); } catch (e) { out("#k", nm(e)); }',
            'class B { constructor() { return {k: 1}; } } class C extends B { #y = 2; static g(o) { return o.#y; } } var c = new C(); out("#k", [C.g(c), c.k]);',
            'class A { static { this.v = %s; } static [Symbol.iterator]() {} } out("#k", A.v);' % self.p(),
            'var l = []; class A { [(l.push(1), "a")] = l.push(2); [(l.push(3), "b")] = 4; } new A(); out("#k", l);',
            'class A { constructor() { this.v = new.target === A; } } class B extends A { constructor() { var f = () => super(); f(); } } out("#k", new B().v);',
            'var p = new Proxy({}, {}); class A { #x = 1; static g(o) { return #x in o; } } out("#k", A.g(p));',
        ])
        return 'classe', x

    def bloco_desestruturar(self):
        x = self.e([
            'var l = []; var [a = l.push(1), , ...r] = [undefined, 2, 3, 4]; out("#k", [a, r, l]);',
            'var {a, b: {c} = {c: %s}, ...z} = {a: 1, d: 2, e: 3}; out("#k", [a, c, z]);' % self.p(),
            'var it = { [Symbol.iterator]() { var i = 0; return { next() { return {done: i > 2, value: i++}; }, return() { out("#k", "fechou"); return {}; } }; } }; var [x, y] = it; out("#k", [x, y]);',
            'function f(a, b = a + 1, ...c) { return [a, b, c, arguments.length]; } out("#k", f(%s));' % ', '.join(self.r.sample(PEQ, self.r.randint(0, 4))),
            'out("#k", [...[1, , 3], ...new Set([1, 1, 2]), ..."ab"]);',
            'out("#k", Math.max(...[%s]));' % ', '.join(self.r.sample(INTS, 3)),
            'var o = {...{a: 1, get b() { return 2; }}, ...null, ...[9]}; out("#k", o);',
        ])
        return 'desestruturar', x

    def bloco_controle(self):
        x = self.e([
            'var s = []; fora: for (var i = 0; i < 4; i++) { for (var j = 0; j < 4; j++) { if (j === %s) continue fora; if (i === %s) break fora; s.push(i * 10 + j); } } out("#k", s);' % (self.e(['1', '2']), self.e(['2', '3'])),
            'var fs = []; for (let i = 0; i < 3; i++) { fs.push(() => i); } var gs = []; for (var j = 0; j < 3; j++) { gs.push(() => j); } out("#k", [fs.map(f => f()), gs.map(f => f())]);',
            'function f() { try { return 1; } finally { %s } } out("#k", f());' % self.e(['', 'return 2;', 'out("#k", "fin");']),
            'var l = []; function* g() { try { yield 1; yield 2; } finally { l.push("f"); } } for (var v of g()) { l.push(v); %s } out("#k", l);' % self.e(['break;', '', 'continue;']),
            'function* g() { var x = yield 1; try { yield x; } catch (e) { yield "c" + e; } } var it = g(); out("#k", [it.next().value, it.next(5).value, it.throw(7).value, it.return(9).value, it.next().done]);',
            'var n = 0; do { n++; if (n > 5) break; } while (n %% %s); out("#k", n);' % self.e(['2', '3']),
            'out("#k", (function () { switch (%s) { case 0: return "z"; case 1: case "1": return "u"; default: return "d"; } })());' % self.p(),
        ])
        return 'controle', x

    def bloco_async(self):
        x = self.e([
            'Promise.resolve(%s).then(v => out("A#k", v)); out("#k", "sinc");' % self.p(),
            'async function f() { await null; out("A#k", 1); throw new RangeError(); } f().catch(e => out("A#k", nm(e)));',
            'Promise.all([1, Promise.resolve(2), {then(r) { r(3); }}]).then(v => out("A#k", v));',
            'Promise.allSettled([Promise.reject(new TypeError()), 1]).then(v => out("A#k", v.map(x => x.status)));',
            'Promise.any([Promise.reject(1), Promise.reject(2)]).catch(e => out("A#k", [nm(e), e.errors]));',
            'async function* g() { yield 1; yield await Promise.resolve(2); } (async () => { var l = []; for await (var v of g()) l.push(v); out("A#k", l); })();',
            'var l = []; Promise.resolve().then(() => l.push(1)); Promise.reject(2).catch(v => l.push(v)); Promise.resolve().then(() => out("A#k", l));',
        ])
        return 'async', x

    def bloco_colecoes(self):
        x = self.e([
            'var m = new Map([[1, "a"], [2, "b"], [3, "c"]]); var l = []; for (var [k] of m) { l.push(k); if (k === 1) { m.delete(2); m.set(4, "d"); } } out("#k", l);',
            'var s = new Set([1, 2, 3]); var l = []; s.forEach(v => { l.push(v); if (v < 3) s.add(v + 10); if (v === 2) s.clear(); }); out("#k", l);',
            'var m = new Map(); m.set(-0, 1); m.set(NaN, 2); m.set(NaN, 3); out("#k", [m.get(0), m.get(NaN), m.size, [...m.keys()]]);',
            'out("#k", Object.groupBy([1, 2, 3, 4], x => x %% %s ? "i" : "p"));' % self.e(['2', '3']),
            'var w = new WeakMap(); var k = {}; w.set(k, 1); out("#k", [w.get(k), w.has({})]);',
        ])
        return 'colecoes', x

    def bloco_grande(self):
        # Tamanhos perto do limite: o que trava ou estoura memória em motores.
        x = self.e([
            'var a = []; a.length = 4294967295; out("#k", a.%s);' % self.e(['indexOf(1)', 'includes(1)', 'lastIndexOf(1)', 'at(-1)']),
            'var o = {length: %s}; out("#k", Array.prototype.indexOf.call(o, 1, 4294967290));' % self.e(['4294967296', '9007199254740991']),
            'try { new Array(%s); out("#k", "ok"); } catch (e) { out("#k", nm(e)); }' % self.e(['4294967296', '-1', '1.5', '4294967295']),
            'try { "x".repeat(%s); out("#k", "ok"); } catch (e) { out("#k", nm(e)); }' % self.e(['2 ** 30', '2 ** 28', '-1']),
            'var a = [1]; try { a.length = %s; out("#k", a.length); } catch (e) { out("#k", nm(e)); }' % self.e(['4294967296', '2 ** 32 - 1', '-1']),
            'var o = {length: 2 ** 53 - 1}; try { Array.prototype.push.call(o, 1); } catch (e) { out("#k", nm(e)); } out("#k", o.length);',
        ])
        return 'grande', x

    BLOCOS = ['bloco_array', 'bloco_mutacao', 'bloco_sort', 'bloco_getter_proto', 'bloco_especie', 'bloco_proxy', 'bloco_tipado',
              'bloco_inteiros', 'bloco_textos', 'bloco_regex', 'bloco_json', 'bloco_classe', 'bloco_desestruturar', 'bloco_controle',
              'bloco_async', 'bloco_colecoes', 'bloco_grande']
    PESOS = [8, 6, 3, 4, 3, 5, 8, 6, 5, 5, 4, 4, 4, 4, 3, 3, 1]

    def programa(self):
        n = self.r.randint(6, 16)
        blocos = []
        for k in range(n):
            tipo, codigo = getattr(self, self.r.choices(self.BLOCOS, self.PESOS)[0])()
            codigo = codigo.replace('"#k"', '"#%d"' % k).replace('"A#k"', '"A#%d"' % k)
            blocos.append('/*%s*/ try { %s } catch (e) { out("#%d!", nm(e)); }' % (tipo, codigo, k))
        return blocos


def montar(blocos):
    return PRELUDIO + '\n' + '\n'.join(blocos) + '\n'

# ---------------------------------------------------------------- execução

def limite():
    resource.setrlimit(resource.RLIMIT_DATA, (2 << 30, 2 << 30))
    os.nice(10)


def esperar_memoria():
    # Não abre processo novo com menos de 3 GB disponíveis: os casos de memória chegam a 2 GB.
    while True:
        try:
            with open('/proc/meminfo') as f:
                livre = next(int(l.split()[1]) for l in f if l.startswith('MemAvailable'))
        except Exception:
            return
        if livre > 3 << 20:
            return
        time.sleep(2)


def rodar(cmd, arq, env=None):
    esperar_memoria()
    ini = time.monotonic()
    try:
        p = subprocess.run(cmd + [arq], capture_output=True, timeout=20, preexec_fn=limite, env=env)
        rc, saida, erro = p.returncode, p.stdout.decode('utf-8', 'replace'), p.stderr.decode('utf-8', 'replace')
    except subprocess.TimeoutExpired:
        rc, saida, erro = 'TEMPO', '', ''
    return {'rc': rc, 'saida': saida[-20000:], 'erro': erro[-4000:], 't': time.monotonic() - ini}


class Executor:
    def __init__(self, falcao, ref, duplo):
        self.falcao, self.ref, self.duplo = [falcao], ref, duplo
        self.env_ref = dict(os.environ, LD_LIBRARY_PATH=os.path.dirname(os.path.abspath(ref))) if ref else None
        # 'tudo': compila toda função e laço na primeira vez (o máximo de código no N2).
        self.env_jit = dict(os.environ, FALCAO_JIT='tudo')

    def par(self, fonte):
        with tempfile.NamedTemporaryFile('w', suffix='.js', delete=False, encoding='utf-8') as f:
            f.write(fonte)
            arq = f.name
        try:
            a = rodar(self.falcao, arq)
            b = rodar(self.falcao, arq, self.env_jit) if self.duplo else rodar([self.ref], arq, self.env_ref)
        finally:
            os.unlink(arq)
        return a, b


def panico(r):
    m = re.search(r"panicked at ([^\n]+?):(\d+):\d+", r['erro'])
    return '%s:%s' % (m.group(1), m.group(2)) if m else None


def sem_memoria(r):
    return 'memory allocation of' in r['erro'] or 'Out of memory' in r['saida'] or 'capacity overflow' in r['erro']


# Diferenças permitidas (definidas pela implementação ou só de texto).
PERMITIDAS = [re.compile(p) for p in [
    r'^ERRO_JS ', r'^uncaught exception', r'^Warning', r'^\s*$',
]]


def linhas(s):
    return [l for l in s.split('\n') if not any(p.match(l) for p in PERMITIDAS)]


def classificar(a, b):
    """(classe, chave) com a = Falcão, b = referência (ou o Falcão com FALCAO_JIT=tudo)."""
    ok_b = b['rc'] == 0 and b['t'] < 1.0
    if a['rc'] == 'TEMPO' or 'Error: prazo' in a['saida']:
        return ('HANG', 'prazo') if ok_b else ('LENTO_NOS_DOIS', '')
    if sem_memoria(a):
        return ('OOM', 'memoria') if b['rc'] == 0 else ('MEMORIA_NOS_DOIS', '')
    if a['rc'] == 4 or (isinstance(a['rc'], int) and (a['rc'] < 0 or a['rc'] >= 128)):
        return ('CRASH', panico(a) or 'sinal %s' % a['rc'])
    if b['rc'] != 0 and 'SyntaxError' in (b['saida'] + b['erro']):
        return ('INVALIDO', '')
    if b['rc'] == 'TEMPO':
        return ('REF_LENTA', '')
    la, lb = linhas(a['saida']), linhas(b['saida'])
    if la == lb:
        return ('OK', '')
    for x, y in zip(la + [''] * len(lb), lb + [''] * len(la)):
        if x != y:
            return ('DIVERGENCIA', re.sub(r'#\d+', '#', x)[:120] + ' || ' + re.sub(r'#\d+', '#', y)[:120])
    return ('OK', '')

# ---------------------------------------------------------------- redução

def reduzir(ex, blocos, classe, chave, orcamento):
    """Delta debugging sobre os blocos e depois sobre os comandos do bloco; mantém a classe."""
    gastos = [0]

    def mantem(bs):
        if gastos[0] >= orcamento or not bs:
            return False
        gastos[0] += 1
        c, k = classificar(*ex.par(montar(bs)))
        if c != classe:
            return False
        return classe != 'CRASH' or k == chave

    n = 2
    while len(blocos) >= 2 and gastos[0] < orcamento:
        pedaco = max(1, len(blocos) // n)
        tirou = False
        for i in range(0, len(blocos), pedaco):
            resto = blocos[:i] + blocos[i + pedaco:]
            if mantem(resto):
                blocos, n, tirou = resto, max(n - 1, 2), True
                break
        if not tirou:
            if pedaco == 1:
                break
            n = min(len(blocos), n * 2)
    # Dentro de cada bloco: tira comandos `;` um a um (só se o resto continua válido e mantém).
    for bi in range(len(blocos)):
        partes = blocos[bi].split('; ')
        i = 0
        while i < len(partes) and len(partes) > 1 and gastos[0] < orcamento:
            tentativa = partes[:i] + partes[i + 1:]
            nb = blocos[:bi] + ['; '.join(tentativa)] + blocos[bi + 1:]
            if mantem(nb):
                partes = tentativa
                blocos = nb
            else:
                i += 1
    return blocos, gastos[0]


def forma(blocos):
    return '+'.join(sorted(set(re.findall(r'/\*(\w+)\*/', ' '.join(blocos)))))

# ---------------------------------------------------------------- laço principal

def principal(a):
    os.makedirs(a.saida, exist_ok=True)
    for d in ('achados', 'corpus'):
        os.makedirs(os.path.join(a.saida, d), exist_ok=True)
    ex = Executor(a.falcao, a.ref, a.duplo)
    estat = {}
    vistos = {}
    trava = threading.Lock()
    fim = time.monotonic() + a.minutos * 60
    semente0 = a.semente if a.semente is not None else int(time.time())
    total = [0]
    log = open(os.path.join(a.saida, 'log.txt'), 'a')

    def um(i):
        if time.monotonic() > fim:
            return
        semente = semente0 * 1_000_003 + i
        blocos = Gerador(semente).programa()
        ra, rb = ex.par(montar(blocos))
        classe, chave = classificar(ra, rb)
        with trava:
            total[0] += 1
            estat[classe] = estat.get(classe, 0) + 1
            if i % 500 == 0:
                with open(os.path.join(a.saida, 'corpus', 'amostra_%d.js' % i), 'w') as f:
                    f.write(montar(blocos))
        if classe in ('OK', 'INVALIDO', 'LENTO_NOS_DOIS', 'MEMORIA_NOS_DOIS', 'REF_LENTA'):
            return
        # Primeira deduplicação, antes de reduzir: mesma classe e chave já vista 3 vezes basta.
        bruto = (classe, chave if classe != 'DIVERGENCIA' else re.sub(r'[-\d.e+]+', 'N', chave)[:80])
        with trava:
            vistos[bruto] = vistos.get(bruto, 0) + 1
            if vistos[bruto] > 3:
                return
        orc = {'CRASH': 300, 'HANG': 25, 'OOM': 40, 'DIVERGENCIA': 120}[classe]
        menor, gastos = reduzir(ex, blocos, classe, chave, orc)
        ra2, rb2 = ex.par(montar(menor))
        c2, k2 = classificar(ra2, rb2)
        id_ = '%s_%s' % (classe, hashlib.sha1((c2 + k2 + forma(menor)).encode()).hexdigest()[:10])
        caminho = os.path.join(a.saida, 'achados', id_)
        with trava:
            novo = not os.path.exists(caminho + '.js')
            if novo:
                with open(caminho + '.js', 'w') as f:
                    f.write(montar(menor))
                with open(caminho + '.json', 'w') as f:
                    json.dump({'classe': c2, 'chave': k2, 'forma': forma(menor), 'semente': semente, 'reducoes': gastos,
                               'falcao': ra2, 'referencia': rb2, 'blocos': menor}, f, indent=1, ensure_ascii=False)
            log.write('%s %s %s %s\n' % (time.strftime('%H:%M:%S'), id_, 'novo' if novo else 'repetido', k2[:150]))
            log.flush()

    i = 0
    with cf.ThreadPoolExecutor(a.trabalhadores) as pool:
        pendentes = set()
        while time.monotonic() < fim and i < a.max:
            while len(pendentes) >= a.trabalhadores * 2:
                feitos, pendentes = cf.wait(pendentes, return_when=cf.FIRST_COMPLETED)
                for f in feitos:
                    f.result()
            pendentes.add(pool.submit(um, i))
            i += 1
            if i % 1000 == 0:
                with trava:
                    with open(os.path.join(a.saida, 'estatisticas.json'), 'w') as f:
                        json.dump({'programas': total[0], 'classes': estat, 'semente': semente0}, f)
        cf.wait(pendentes)
    with open(os.path.join(a.saida, 'estatisticas.json'), 'w') as f:
        json.dump({'programas': total[0], 'classes': estat, 'semente': semente0, 'agrupados': {'%s|%s' % k: v for k, v in vistos.items()}},
                  f, indent=1, ensure_ascii=False)
    print(json.dumps({'programas': total[0], 'classes': estat}, ensure_ascii=False))


def resumo(dir_):
    """Agrupa os achados reduzidos: classe, chave normalizada e os tipos de bloco envolvidos."""
    grupos = {}
    pasta = os.path.join(dir_, 'achados')
    for nome in sorted(os.listdir(pasta)):
        if not nome.endswith('.json'):
            continue
        d = json.load(open(os.path.join(pasta, nome)))
        k = (d['classe'], d['forma'], re.sub(r'-?\d+(\.\d+)?(e[-+]?\d+)?', 'N', d['chave'])[:90])
        grupos.setdefault(k, []).append(nome[:-5])
    for (c, f, k), ids in sorted(grupos.items(), key=lambda x: (['CRASH', 'OOM', 'HANG', 'DIVERGENCIA'].index(x[0][0]), -len(x[1]))):
        print('%-11s %3d  %-14s %s  ex: %s' % (c, len(ids), f, k, ids[0]))
    est = os.path.join(dir_, 'estatisticas.json')
    if os.path.exists(est):
        e = json.load(open(est))
        print('programas:', e['programas'], 'classes:', e['classes'])


def teste():
    # O gerador é determinístico pela semente e o redutor acha o bloco mínimo.
    assert Gerador(7).programa() == Gerador(7).programa()
    assert any('/*tipado*/' in b for s in range(30) for b in Gerador(s).programa())

    class Falso:
        def par(self, fonte):
            ruim = 'BUG' in fonte
            a = {'rc': 4 if ruim else 0, 'saida': '', 'erro': "thread 'x' panicked at src/a.rs:1:2" if ruim else '', 't': 0}
            return a, {'rc': 0, 'saida': '', 'erro': '', 't': 0}
    blocos = ['x%d;' % i for i in range(20)]
    blocos[13] = 'y; BUG; z;'
    menor, _ = reduzir(Falso(), blocos, 'CRASH', 'src/a.rs:1', 500)
    assert menor == ['BUG;'] or menor == ['BUG'], menor
    print('ok')


if __name__ == '__main__':
    ap = argparse.ArgumentParser()
    ap.add_argument('--falcao')
    ap.add_argument('--ref')
    ap.add_argument('--saida')
    ap.add_argument('--minutos', type=float, default=90)
    ap.add_argument('--max', type=int, default=100_000)
    ap.add_argument('--trabalhadores', type=int, default=6)
    ap.add_argument('--semente', type=int)
    ap.add_argument('--duplo', action='store_true')
    ap.add_argument('--teste', action='store_true')
    ap.add_argument('--resumo', help='pasta de saída de uma rodada: agrupa os achados')
    a = ap.parse_args()
    if a.teste:
        teste()
    elif a.resumo:
        resumo(a.resumo)
    else:
        principal(a)
