# Falcão: da árvore sintática ao bytecode e à execução

Pesquisa para desenhar o bytecode do Falcão (motor de JavaScript em Rust). Escopo: o que cada operação, expressão e comando faz em tempo de execução, segundo a norma, para orientar as instruções do bytecode.

## 0. Fontes, estado e limites desta pesquisa

- Fonte: ECMAScript em https://tc39.es/ecma262/ (versão de rascunho indicada no próprio sumário como "ECMAScript 2027"; consultada em 18/10/2026) e, para exemplos de uso, MDN em pt-BR.
- Nenhum código ou lista de bytecode de outros motores foi aberto ou consultado.
- **Limite importante:** a ferramenta de leitura da norma só devolveu o sumário e trechos de índice; o corpo dos algoritmos não foi extraído nesta sessão. Por isso ficam marcados dois níveis:
  - **[confirmado]**: número de seção conferido no sumário oficial (página acessada nesta sessão).
  - **[memória]**: passos escritos a partir do conhecimento da norma. Devem ser conferidos no texto de tc39.es, item por item, antes de virar código.
- Contagens de métodos dos objetos embutidos (seção 7 deste documento) **não foram levantadas**. Não preenchi número de memória para não inventar dado.
- Números de seção da cláusula 7 [confirmado]: o sumário dá a numeração atual. Algumas numerações da lista original do pedido estavam deslocadas (ex.: ToString é 7.1.19, não 7.1.17). Este documento usa a numeração do sumário.

---

## 1. Operações abstratas essenciais (cláusula 7 e 6.2)

### 1.1 Conversões (7.1)

**ToPrimitive(input, preferredType) — 7.1.1 [confirmado número; passos memória]**
1. Se `input` não é objeto, retornar `input`.
2. `exoticToPrim = GetMethod(input, @@toPrimitive)`.
3. Se existe: `hint` = "default", "number" ou "string" (conforme `preferredType`); `result = Call(exoticToPrim, input, hint)`; se `result` não é objeto, retornar; senão TypeError.
4. Senão: `OrdinaryToPrimitive(input, "number" ou "string"; "default" vira "number")`.

**OrdinaryToPrimitive(O, hint) — 7.1.1.1 [confirmado]**
1. Se hint é "string": tentar `toString` e depois `valueOf`. Senão: `valueOf` e depois `toString`.
2. Para cada método: se `IsCallable`, chamar; se o resultado não é objeto, retornar.
3. Senão TypeError.

**ToBoolean(arg) — 7.1.2 [confirmado número]**
- Falso para: `undefined`, `null`, `false`, `+0`, `-0`, `NaN`, `0n`, `""`, e objeto marcado como "document.all" (caso de host).
- Qualquer outro valor (inclusive objetos e símbolos) é verdadeiro.

**ToNumeric(arg) — 7.1.3 [confirmado número]**
1. `prim = ToPrimitive(arg, number)`.
2. Se `prim` é BigInt, retornar `prim`.
3. Retornar `ToNumber(prim)`.

**ToNumber(arg) — 7.1.4 [confirmado número]**
- `undefined` gives NaN; `null` gives +0; `true` gives 1; `false` gives +0.
- Número: retorna igual.
- BigInt: TypeError (não converte implicitamente).
- Símbolo: TypeError.
- String: `StringToNumber` (aceita vazio/espaços como 0, `0x`, `0o`, `0b`, `Infinity`; qualquer outra coisa vira NaN).
- Objeto: `ToNumber(ToPrimitive(arg, number))`.

**ToIntegerOrInfinity(arg) — 7.1.5 [confirmado número]**
1. `n = ToNumber(arg)`.
2. NaN ou ±0 gives 0. ±∞ fica como está.
3. Senão, truncar para inteiro (em direção a zero).

ToInt32 e ToUint32 (7.1.6 e 7.1.7) e ToInt8/ToUint8/ToInt16/ToUint16/ToBigInt64/ToBigUint64 (7.1.8 a 7.1.15) [memória para os números] fazem a mesma coisa com módulo 2^n; são usados pelos operadores de bit a bit e pelos TypedArrays.

**ToString(arg) — 7.1.19 [confirmado número]**
- Símbolo: TypeError.
- Objeto: `ToString(ToPrimitive(arg, string))`.
- Número: `Number::toString` (13.2.6 de memória); BigInt e demais: representação textual.

**ToObject(arg) — 7.1.20 [confirmado número]**
- `undefined` e `null`: TypeError.
- Boolean, Number, String, Symbol, BigInt: cria o objeto empacotador (wrapper) correspondente.
- Objeto: retorna o próprio.

**ToPropertyKey(arg) — 7.1.21 [confirmado número]**
1. `key = ToPrimitive(arg, string)`.
2. Se `key` é símbolo, retornar `key`.
3. Retornar `ToString(key)`.

**ToLength(arg) — 7.1.22 [confirmado número]**
1. `len = ToIntegerOrInfinity(arg)`.
2. Se `len <= 0`, retornar 0.
3. Retornar `min(len, 2^53 - 1)`.

### 1.2 Testes e comparações (7.2)

**IsArray(arg) — 7.2.2 [confirmado número]**: se é Proxy revogado, TypeError; se é Proxy, olha o alvo; senão, é exótico Array.

**IsCallable(arg) — 7.2.3 [confirmado]**: é objeto com slot [[Call]].

**IsConstructor(arg) — 7.2.4 [confirmado]**: é objeto com slot [[Construct]].

**SameValue(x, y) — 7.2.9 [confirmado número]**
1. Tipos diferentes: falso.
2. Número: NaN iguala NaN; +0 e -0 são diferentes; senão comparação numérica.
3. Demais tipos: igualdade por valor (string, BigInt, booleano) ou por identidade (objeto, símbolo).

**SameValueZero(x, y) — 7.2.10 [confirmado número]**: igual a SameValue, mas +0 e -0 são iguais. Usada por `Array.prototype.includes` e `Map`/`Set`.

**IsLessThan(x, y, LeftFirst) — 7.2.12 [confirmado número]**
1. Converte `x` e `y` com ToPrimitive(number), na ordem indicada por `LeftFirst`.
2. Se ambos são string: comparação por unidades de código.
3. Se um é BigInt e outro string: converte a string com StringToBigInt (se falhar, indefinido).
4. Senão, converte com ToNumeric e compara; NaN ou indefinido resulta em "não menor" (retorna falso no operador `<`).

**IsStrictlyEqual(x, y) — 7.2.14 [confirmado número]**
1. Tipos diferentes: falso.
2. Número: igualdade numérica (NaN não iguala NaN; +0 iguala -0).
3. String, BigInt, booleano: igualdade por valor.
4. `undefined`, `null`: verdadeiro.
5. Objeto e símbolo: identidade.

**IsLooselyEqual(x, y) — 7.2.13 [confirmado número]**
1. Mesmo tipo: `IsStrictlyEqual`.
2. `null` com `undefined` (qualquer ordem): verdadeiro.
3. Número com string: `x == ToNumber(y)`.
4. BigInt com string: converte a string com StringToBigInt; se falhar, falso.
5. Booleano em qualquer lado: converte com ToNumber e compara de novo.
6. Objeto com primitivo (não símbolo/indefinido): `ToPrimitive(objeto)` e compara de novo.
7. BigInt com número: compara matematicamente (sem arredondar).
8. Senão, falso.

**OrdinaryHasInstance(C, O) — 7.3.21 [confirmado número]**
1. `C` não é chamável: falso.
2. `C` é função ligada (bound): `InstanceofOperator(O, BoundTargetFunction)`.
3. `O` não é objeto: falso.
4. `P = Get(C, "prototype")`; se `P` não é objeto: TypeError.
5. Loop: `O = O.[[GetPrototypeOf]]()`; se nulo, falso; se `SameValue(P, O)`, verdadeiro.

### 1.3 Operações sobre objetos (7.3)

**Get(O, P) — 7.3.2 [confirmado]**: `O.[[Get]](P, O)`.

**GetV(V, P) — 7.3.3 [confirmado]**: converte o primitivo para objeto empacotador (sem alocar, na prática) e chama `[[Get]]` com o valor original como receiver.

**Set(O, P, V, Throw) — 7.3.4 [confirmado]**: `O.[[Set]](P, V, O)`; se o resultado é falso e `Throw` é verdadeiro, TypeError.

**CreateDataProperty(O, P, V) — 7.3.5 [confirmado]**: `O.[[DefineOwnProperty]](P, {value, writable, enumerable, configurable: todos verdadeiros})`.

**CreateDataPropertyOrThrow(O, P, V) — 7.3.6 [memória]**: igual, mas TypeError se falhar.

**DefinePropertyOrThrow(O, P, desc) — 7.3.8 [confirmado]**: `[[DefineOwnProperty]]`; falha gera TypeError.

**DeletePropertyOrThrow(O, P) — 7.3.9 [confirmado]**: `[[Delete]]`; falso gera TypeError.

**GetMethod(V, P) — 7.3.10 [confirmado]**
1. `func = GetV(V, P)`.
2. Se `func` é `undefined` ou `null`: retornar `undefined`.
3. Se não é chamável: TypeError.
4. Retornar `func`.

**HasProperty(O, P) — 7.3.11 [confirmado]**: `O.[[HasProperty]](P)`. Base do operador `in`.

**Call(F, V, args) — 7.3.13 [confirmado]**: se `F` não é chamável, TypeError; senão `F.[[Call]](V, args)`.

**Construct(F, args, newTarget) — 7.3.14 [confirmado]**: `F.[[Construct]](args, newTarget)`.

**InvokeMethod / Invoke(V, P, args) — 7.3.x [memória]**: `GetV` seguido de `Call` com `V` como `this`.

### 1.4 Iteradores (7.4)

**GetIterator(obj, kind) — 7.4.4 [confirmado número]**
1. `kind` sync: `method = GetMethod(obj, @@iterator)`; async: tenta `@@asyncIterator` e, se indefinido, usa o sync com `CreateAsyncFromSyncIterator`.
2. Se `method` é indefinido: TypeError.
3. `iterator = Call(method, obj)`; se não é objeto: TypeError.
4. `nextMethod = Get(iterator, "next")`.
5. Retornar `Iterator Record { [[Iterator]], [[NextMethod]], [[Done]] = false }`.

**IteratorStep / IteratorStepValue — 7.4.9 e seguintes [confirmado número para IteratorStep]**
1. `result = Call(nextMethod, iterator)`; se não é objeto: TypeError.
2. `done = ToBoolean(Get(result, "done"))`.
3. Se `done`: marca `[[Done]] = true`, sinaliza fim.
4. Senão, retorna `Get(result, "value")`.

**IteratorClose(iteratorRecord, completion) — 7.4.11 [confirmado número]**
1. `return = GetMethod(iterator, "return")`.
2. Se `undefined`: devolve `completion` original.
3. `innerResult = Call(return, iterator)`.
4. Se `completion` é throw: devolve `completion` (o erro original tem prioridade, o erro de `return` é descartado).
5. Se `innerResult` é throw: propaga.
6. Se `innerResult` não é objeto: TypeError.
7. Devolve `completion`.

**AsyncIteratorClose — 7.4.x [memória]**: igual, mas com `Await` no resultado de `return`.

### 1.5 Referências e completions (6.2)

**Reference Record — 6.2.5 [confirmado como cláusula]**: `base` (valor, ambiente ou "unresolvable"), `referencedName`, `strict`, `thisValue`.

**GetValue(V) — 6.2.5.5 [confirmado número]**
1. Se `V` não é referência: retorna `V`.
2. Se "unresolvable": ReferenceError.
3. Se é referência a propriedade: `ToObject(base)` (quando é primitivo) e `base.[[Get]](name, thisValue)`.
4. Se é referência a ambiente: `base.GetBindingValue(name, strict)`.

**PutValue(V, W) — 6.2.5.6 [confirmado número]**
1. Se `V` é "unresolvable": em modo estrito ReferenceError; senão cria propriedade no global.
2. Se é propriedade: `base = ToObject(base)`; `succeeded = base.[[Set]](name, W, thisValue)`; se falso e estrito, TypeError.
3. Se é ambiente: `base.SetMutableBinding(name, W, strict)`.

**Completion Record — 6.2.4 [confirmado]**: tipos `normal`, `break`, `continue`, `return`, `throw`; construtores `NormalCompletion`, `ThrowCompletion`, `ReturnCompletion`, `UpdateEmpty` (6.2.4.1 a 6.2.4.4).

### 1.6 Outras operações de execução

**Await(value) — 6.2.3.x [memória]**: suspende o contexto (ver seção 5.3); resolve o valor com `PromiseResolve`, registra reações `onFulfilled` e `onRejected` que retomam o contexto.

**typeof (tabela) — 13.5.3 [memória do número]**

| Valor | Resultado de `typeof` |
|---|---|
| undefined | "undefined" |
| null | "object" (erro histórico, mantido) |
| booleano | "boolean" |
| número | "number" |
| BigInt | "bigint" |
| string | "string" |
| símbolo | "symbol" |
| objeto chamável (tem [[Call]]) | "function" |
| outro objeto | "object" |
| referência não resolvida (identificador inexistente) | "undefined" (sem ReferenceError) |

Confirmado pelo MDN em pt-BR, que traz a mesma tabela; BigInt e Symbol aparecem na norma.

---

## 2. Semântica de execução de expressões (cláusula 13)

Em todos os itens, a ordem é: avalia operandos da esquerda para a direita (5.2.1 [confirmado número]); converte; opera; devolve. Números de seção de operadores [confirmados pelo sumário]: 13.3 esquerda, 13.4 atualização, 13.5 unários, 13.6 exponenciação, 13.7 multiplicativos, 13.8 aditivos, 13.9 deslocamento, 13.10 relacionais (13.10.2 InstanceofOperator), 13.11 igualdade, 13.12 bit a bit, 13.13 lógicos, 13.14 condicional, 13.15 atribuição, 13.16 vírgula. Passos abaixo são [memória].

### 2.1 Aritméticos (13.6 a 13.8)

**`+`, `-`, `*`, `/`, `%` (Number)**
1. Avalia `lval`, depois `rval` (GetValue de cada um).
2. `lnum = ToNumeric(lval)`, `rnum = ToNumeric(rval)` (nesta ordem).
3. Se os dois tipos diferem (um BigInt e outro Number): TypeError.
4. Se são Number: IEEE 754 binário64 (NaN, infinitos, zeros com sinal). `%` usa resto com sinal do dividendo.
5. Se são BigInt: aritmética inteira; `/` trunca em direção a zero; `%` com sinal do dividendo; divisão por zero: RangeError.

**`+` com string**: se `lprim` ou `rprim` é string, concatena `ToString` dos dois; senão soma numérica (ou BigInt). Antes disso, `ToPrimitive` dos dois operandos com hint default.

**`**` (13.6)**: base e expoente com ToNumeric; tipos diferentes: TypeError; BigInt com expoente negativo: RangeError; Number segue `Number::exponentiate` (casos especiais: `NaN ** 0` é 1, `1 ** Infinity` é NaN).

### 2.2 Bit a bit e deslocamento (13.9, 13.12)

- `&`, `|`, `^`, `<<`, `>>`, `>>>`: `ToNumeric` dos dois operandos; tipos diferentes: TypeError.
- Number: `ToInt32` (e `ToUint32` para `>>>`); o deslocamento usa só os 5 bits menores do contador.
- BigInt: `>>>` lança TypeError; deslocamento com expoente negativo muda de direção; operação com complemento de dois infinito.
- `~` (unário): `ToNumeric` e depois `~x` (Number usa ToInt32; BigInt usa `-x - 1`).

### 2.3 Unários (13.5)

- `-x`: `ToNumeric`; Number nega; BigInt nega.
- `+x`: `ToNumber(x)`; BigInt lança TypeError.
- `!x`: `ToBoolean(x)` negado.
- `void x`: avalia `x`, devolve `undefined`.
- `typeof x`: tabela da seção 1.6. Se `x` é identificador não resolvido, devolve "undefined" sem erro.
- `delete x`: (ver 2.6).

### 2.4 Comparação relacional (13.10)

- `a < b`: `IsLessThan(a, b, true)`; se indefinido, falso.
- `a > b`: `IsLessThan(b, a, false)`; indefinido, falso.
- `a <= b`: `IsLessThan(b, a, false)`; se verdadeiro ou indefinido, falso; senão, verdadeiro.
- `a >= b`: `IsLessThan(a, b, true)`; se verdadeiro ou indefinido, falso; senão, verdadeiro.
- `a instanceof b` (13.10.2 [confirmado]): se `b` não é objeto: TypeError; `h = GetMethod(b, @@hasInstance)`; se existe: `ToBoolean(Call(h, b, a))`; senão, verifica `IsCallable(b)` (TypeError se não) e usa `OrdinaryHasInstance(b, a)`.
- `#x in obj` (13.10 [memória]): `obj` deve ser objeto (TypeError senão); verifica se o nome privado `#x` está no objeto (sem passar pelo `[[Get]]`).
- `a in b`: se `b` não é objeto: TypeError; `HasProperty(b, ToPropertyKey(a))`.

### 2.5 Igualdade (13.11)

- `a === b`: `IsStrictlyEqual(a, b)`.
- `a !== b`: negação de `===`.
- `a == b`: `IsLooselyEqual(a, b)`.
- `a != b`: negação de `==`.
- `Object.is` não é operador (é função, ver seção 7); usa `SameValue`.

### 2.6 Atribuição, ++/--, delete (13.4, 13.15, 13.5)

**Atribuição simples `a = b` (13.15.2)**
1. Avalia a referência de `a` (sem ler o valor). Para destruturação, vai para o padrão.
2. Avalia `b` e faz GetValue.
3. Se `a` é identificador anônimo e `b` é função/classe anônima: dá nome (NamedEvaluation).
4. `PutValue(ref, valor)`; o valor atribuído é o resultado da expressão.

**Composta `a op= b` (13.15.2)**
1. Avalia referência de `a` e lê `lval = GetValue(ref)`.
2. Avalia `b`, `rval = GetValue`.
3. `r = operação(lval, rval)` (mesmas regras do operador `op`).
4. `PutValue(ref, r)`. A referência é avaliada uma única vez (importante para `a[f()] += 1`).

**Lógica `a &&= b`, `a ||= b`, `a ??= b` (13.15.2)**
1. Lê `lval`.
2. Se o curto-circuito dispensa a atribuição (`&&=` com `lval` falso, `||=` com `lval` verdadeiro, `??=` com `lval` não nulo): devolve `lval` sem avaliar `b` e sem `PutValue`.
3. Senão, avalia `b`, faz NamedEvaluation se preciso, `PutValue`, devolve o valor.

**`++a` / `--a` (13.4.4, 13.4.5)**: `oldValue = ToNumeric(GetValue(ref))`; `newValue = oldValue ± 1` (BigInt: `±1n`); `PutValue`; devolve `newValue`.

**`a++` / `a--` (13.4.2, 13.4.3)**: igual, mas devolve `oldValue` (já convertido com ToNumeric; por isso `"3"++` dá `3`, número).

**`delete ref` (13.5.1.2)**
1. Se a expressão não é referência: avalia e devolve `true`.
2. Se "unresolvable": em estrito é SyntaxError (early error); senão devolve `true` (ou tenta remover do global).
3. Se propriedade: `base = ToObject(base)`; `base.[[Delete]](name)`; em estrito, falso gera TypeError.
4. Se ambiente: `DeleteBinding` (só remove bindings criados por `eval` ou globais configuráveis).

### 2.7 Operadores lógicos e curto-circuito (13.13)

- `a && b`: avalia `a`; se `ToBoolean(a)` é falso, devolve `a` (sem avaliar `b`); senão devolve `b`.
- `a || b`: avalia `a`; se verdadeiro, devolve `a`; senão devolve `b`.
- `a ?? b` (13.13): avalia `a`; se não é `null` nem `undefined`, devolve `a`; senão devolve `b`. Não pode misturar com `&&`/`||` sem parênteses (early error).

### 2.8 Condicional e vírgula (13.14, 13.16)

- `c ? a : b`: `ToBoolean(GetValue(c))`; avalia só o ramo escolhido.
- `a, b`: avalia `a` e descarta (GetValue); devolve `b` (GetValue).

### 2.9 Encadeamento opcional `?.` (13.3.9)

- `a?.b`: avalia `a`; se `null` ou `undefined`, a expressão inteira vira `undefined` e o resto da cadeia (`.c`, `[d]`, `(e)`) não é avaliado. Ao sair do encadeamento, a cadeia é "cortada" para o `?.` mais próximo.
- `a?.()`: se `a` é `null`/`undefined`, curto-circuito; senão `Call` com `this` correto.
- `a?.b.c` com `a` nulo: resultado `undefined`, sem erro.

### 2.10 Acesso a propriedades e chamadas (13.3.2, 13.3.3, 13.3.6)

**Acesso `a.b` / `a[e]` (13.3.2)**
1. Avalia `a` (GetValue).
2. Para `a[e]`: avalia `e`, `GetValue`, `ToPropertyKey`.
3. Se `a` é `null`/`undefined`: TypeError (acontece na leitura, não na montagem da referência).
4. Devolve a Reference Record (base = `a`, nome = chave, `thisValue` = `a`).

**Chamada `f(args)` (13.3.6.1 [memória])**
1. Se a função é identificador: `ref` é a referência; senão ref é o valor.
2. `func = GetValue(ref)`.
3. `thisValue`: se `ref` é propriedade, `thisValue` é a base (`a.b()` chama com `this = a`); se é ambiente com `WithBaseObject` (dentro de `with`), o objeto; senão `undefined`.
4. Avalia os argumentos (com spread, ver 2.12) à esquerda para direita.
5. Se `func` não é chamável: TypeError.
6. `Call(func, thisValue, args)`.

**Eval direto (13.3.6.1 e 19.2.1 [memória])**
- Só vale quando a chamada é `eval(...)` com o identificador `eval` resolvendo para o `%eval%` intrínseco.
- Se o identificador não é `%eval%`, é chamada comum (eval indireto: `(0, eval)(s)` ou `window.eval(s)`).
- Direto: `PerformEval(x, strictCaller, direct=true)` com o ambiente léxico atual; código com `var` cria bindings no ambiente da função (ou global); código estrito cria ambiente próprio.
- Indireto: executa no ambiente global, como script.

**`new C(args)` (13.3.5)**
1. `constructor = GetValue(C)`.
2. Avalia argumentos.
3. Se não é construtor: TypeError.
4. `Construct(constructor, args)`.

**`super.x` e `super[e]` (13.3.7 [memória])**: lê via `[[GetPrototypeOf]]` do `[[HomeObject]]` da função atual, depois `Get` com receiver `this`. `super(args)` (ver 5.2).

### 2.11 Objetos, arrays e templates

**Literal de objeto `{...}` (13.2.5.5 [confirmado]**: `ObjectDefinitionEvaluation`)**
1. Cria objeto ordinário com `%Object.prototype%`.
2. Para cada propriedade, em ordem: avalia chave (ToPropertyKey quando computada), avalia valor, `CreateDataPropertyOrThrow` (ou `DefineMethod`/`DefineGetter`/`DefineSetter` para métodos e acessores).
3. `__proto__: x` (sem colchete, sem atalho): define o protótipo se `x` é objeto ou `null`.
4. Spread `...obj` (2.12): copia propriedades próprias enumeráveis com `CopyDataProperties`.

**Literal de array `[a, , ...b]` (13.2.4.2 [confirmado])**
1. Cria array; comprimento 0.
2. Para cada elemento: elisão incrementa o `length` sem criar propriedade; spread itera o valor com iterador e cria índices; senão `CreateDataProperty` no índice.

**Regex literal `/p/f` (13.2.7 [memória])**: a cada avaliação cria um novo objeto `RegExp` (`RegExpCreate`); o padrão é compilado; erros de sintaxe do padrão são early errors.

**Template literal `` `a${x}b` `` (13.2.8.5 [memória])**
1. Cada parte literal é a string cooked; avalia `x`; `ToString(x)` (sem ToPrimitive em hint "default": usa `string`).
2. Concatena em ordem.
3. Símbolo dentro de template: TypeError.

**Template marcado ``f`a${x}b` `` (13.3.11 e 13.2.8.4 [memória])**
1. `templateObject = GetTemplateObject(TemplateLiteral)`: **cacheado por site de chamada** (um objeto por ponto do código-fonte, não por execução).
2. O objeto tem `raw` (strings brutas) e a própria array com strings cooked (`undefined` se o escape for inválido, no caso de template marcado), ambos congelados.
3. Avalia substituições; chama `f(templateObject, ...valores)`, `this` conforme chamada de membro.

### 2.12 Spread e rest (13.3.8 e 13.2.4 [memória])

- Em chamada e array: obtém iterador do valor (`GetIterator`), chama `IteratorStepValue` até o fim e adiciona cada valor. Erro na iteração chama `IteratorClose`.
- Em objeto: `CopyDataProperties`, que pega só chaves próprias enumeráveis (inclusive símbolos), sem o protótipo.

### 2.13 Funções, classes e `new.target`

- Expressão de função / arrow / método / classe (13.2.6): cria objeto de função (`InstantiateOrdinaryFunctionExpression` etc.); arrow não tem `this`/`arguments`/`new.target` próprios.
- `new.target` (13.3.12): em função chamada com `new`, é o construtor; em chamada normal, `undefined`; em arrow, o da função externa.
- `this` (13.2.1.1 [confirmado]): `ResolveThisBinding` do ambiente; em arrow, vem do ambiente léxico.

### 2.14 Campos privados (13.3.2.1 [memória], 9.2 [confirmado PrivateEnvironment])

- `obj.#x`: resolve o nome privado pelo `PrivateEnvironment` do código; `PrivateGet(obj, #x)` — erro TypeError se o objeto não tem o campo.
- `obj.#x = v`: `PrivateSet` — TypeError se não existe ou é método sem setter.
- `#x in obj`: 2.4.
- Campos privados são guardados em lista própria (`[[PrivateElements]]`), não em `[[OwnPropertyKeys]]`.

### 2.15 Literais simples

- Número, string, booleano, `null`, `undefined` (identificador global imutável), BigInt literal: valor constante.
- Símbolo não tem literal.

---

## 3. Comandos (cláusula 14): controle de fluxo em tempo de execução

### 3.1 Completions e rótulos (6.2.4 e 14.13)

- Cada comando devolve um Completion Record: `normal` (com valor ou vazio), `break`, `continue`, `return`, `throw`, todos com `[[Target]]` opcional (rótulo).
- `UpdateEmpty` (6.2.4.4 [confirmado]): propaga o último valor não vazio dos comandos, que é o que `eval` devolve.
- Um `break` sem rótulo sai do laço/`switch` mais próximo; `break rotulo` sai do comando rotulado. `continue` similar para laços.
- Labelled statement (14.13.3 [confirmado]): se o corpo completa com `break` cujo alvo é esse rótulo, o resultado vira `normal`.
- `return`: completion `return` (14.10.1 [confirmado]); em função com `try ... finally`, o `finally` ainda roda.
- `throw`: completion `throw` (14.14.1 [confirmado]).

### 3.2 Bloco, declarações, if, expressão (14.2, 14.3, 14.4, 14.5, 14.6)

- Bloco (14.2.2 [confirmado]): `BlockDeclarationInstantiation` cria ambiente declarativo novo com `let`/`const`/`class` em TDZ, e funções de bloco; roda os comandos; restaura o ambiente anterior ao sair (com qualquer completion).
- `var`: não cria nada em tempo de execução (já foi hoisted); `var x = e` é `PutValue` com a referência de `x`.
- `let x = e` / `const x = e` (14.3.1.2 [confirmado]): `InitializeReferencedBinding`; sai do TDZ.
- `if` (14.6.2 [confirmado]): `ToBoolean` do teste; executa um dos ramos; sem `else` e teste falso: completion normal `undefined`.

### 3.3 Laços (14.7)

**while (14.7.3.2 [confirmado])**: `LoopContinues`: se o teste falha, sai com valor acumulado; `break` sai; `continue` pula para o teste; outras completions (`return`, `throw`) propagam.

**do-while (14.7.2.2 [confirmado])**: executa o corpo antes do primeiro teste.

**for (14.7.4.2 [confirmado])**
1. Inicialização: se `let`/`const`, cria ambiente de laço.
2. A cada iteração (`CreatePerIterationEnvironment`, 14.7.4.4 [confirmado]): copia as variáveis `let` para um ambiente novo, para que closures capturem o valor daquela iteração.
3. Teste, corpo, incremento, mesma regra de completions que `while`.

**for-in (14.7.5.8 e 14.7.5.9 [confirmado])**
1. Avalia o objeto; se `null`/`undefined`, não entra no laço.
2. `ToObject`.
3. `EnumerateObjectProperties(obj)`: percorre as chaves string enumeráveis do objeto e da cadeia de protótipos, sem repetir (chave ocultada por propriedade de filho não aparece), e ignora chaves removidas durante o laço. Símbolos não entram.
4. Para cada chave, atribui à variável (`var`, `let`, `const` ou alvo de atribuição) e roda o corpo.
5. `break` ou `return` sai do laço; **não há IteratorClose** (não é iterador).

**for-of (14.7.5.5 e 14.7.5.7 [confirmado número])**
1. `iteratorRecord = GetIterator(obj, sync)`.
2. Repete: `IteratorStepValue`; se `done`, sai.
3. Atribui o valor ao alvo (padrão de destructuring se necessário) e roda o corpo.
4. Se o corpo termina com `break`, `return` ou `throw`: `IteratorClose(iteratorRecord, completion)` (7.4.11 [confirmado]). Se o próprio `next` lançou erro, **não** chama `return`.
5. `continue` apenas continua o laço.

**for-await (14.7.5.5 com kind async [memória])**
- Usa `GetIterator(obj, async)`.
- Cada passo faz `Await` do resultado de `next()`.
- Ao sair antecipado: `AsyncIteratorClose`, que faz `Await` do retorno de `return()`.

### 3.4 switch (14.12.2 e 14.12.4 [confirmado])

1. Avalia a expressão.
2. `CaseBlockEvaluation`: avalia os `case` na ordem, comparando com `IsStrictlyEqual`, até achar o primeiro igual; se nenhum casa, vai para `default` se existir.
3. A partir do ponto de entrada, executa os corpos em sequência (fall-through) até um `break` ou o fim do bloco.
4. `break` sem rótulo dentro de `switch` sai do bloco inteiro. Um bloco `switch` tem ambiente próprio.

### 3.5 try/catch/finally (14.15.2 e 14.15.3 [confirmado])

**Semântica de finally (14.15.3 [memória])**
1. Executa `try`; o resultado é `B`.
2. Se `B` é throw e há `catch`: executa `catch` com o valor lançado (ambiente novo para o parâmetro, 14.15.2 [confirmado como CatchClauseEvaluation]); o resultado vira `C`.
3. Se há `finally`: executa; seu `F`.
4. Se `F` é normal, o resultado final é `B` (ou `C`).
5. Se `F` é abrupto (`return`, `break`, `continue`, `throw`), **o `finally` sobrepõe** o resultado anterior: o `return`/`throw` do `finally` ganha.
6. Um `return` dentro do `try` com `finally` normal: o `finally` roda e depois o `return` original é entregue.

**Catch sem parâmetro** (`catch {}`): não cria ambiente para o parâmetro.

### 3.6 with (14.11.2 [confirmado])

1. Avalia o objeto; `ToObject`.
2. Cria ambiente de objeto com `withEnvironment = true` (o objeto vira escopo de identificadores).
3. Executa o corpo nesse ambiente; restaura ao sair.
4. Não é permitido em código estrito (early error).
5. Chamadas dentro de `with` usam o objeto como `this` quando o identificador vem dele (ver 2.10).

### 3.7 Outros

- `debugger`: sem efeito obrigatório na norma (ponto de parada opcional de host).
- Declaração de função: já instanciada no hoisting; no comando, normalmente vazio.
- Declaração de classe: cria a classe e sai do TDZ no ponto do comando.
- Declaração vazia `;`: normal vazio.

---

## 4. Ambientes e escopos (cláusula 9.1 e instanciações)

### 4.1 Environment Records (9.1 [confirmado como 9.1])

Tipos [memória dos subtipos, cláusula 9.1 confirmada]:

| Tipo | Uso | Operações principais (memória) |
|---|---|---|
| Declarativo | `let`, `const`, `class`, parâmetros, blocos, catch | `HasBinding`, `CreateMutableBinding`, `InitializeBinding`, `SetMutableBinding`, `GetBindingValue`, `DeleteBinding` |
| De objeto | `with` e o objeto global (parte) | mesmas, mas leem/gravam propriedades de um objeto; `withEnvironment` decide se o `this` vem dele |
| De função | escopo de função: `this`, `new.target`, objeto `home` | `BindThisValue` (TDZ de `this` em derivadas), `GetThisBinding`, `HasSuperBinding` |
| Global | escopo global: parte declarativa (let/const) + parte de objeto (var, function) | `CreateGlobalVarBinding`, `CreateGlobalFunctionBinding`, `HasLexicalDeclaration` |
| De módulo | módulo ES: bindings importados ponteiam para outro ambiente | `CreateImportBinding` (binding indireto) |

Ambientes privados (9.2 [confirmado]): `PrivateEnvironment` resolve nomes `#x` por classe.

### 4.2 Hoisting

- **var** (14.3.2 e 10.2.11 [memória]): criado no ambiente de função ou global como `undefined` na instanciação, antes de qualquer código.
- **Declarações de função** (10.2.11 e 16.1.7 [memória]): criadas e inicializadas na instanciação (com o objeto da função já pronto), visíveis antes da linha onde estão. Em bloco, há regra de Annex B (B.3.3 [memória]) que também cria `var` de mesmo nome no escopo de função, em modo não estrito.
- **let/const/class**: criados sem valor, em TDZ, na instanciação do bloco ou escopo; acesso antes da inicialização: ReferenceError (`GetBindingValue` com `uninitialized`).

### 4.3 TDZ

- Acesso a `let`/`const`/`class` antes de `InitializeBinding`: ReferenceError.
- `typeof x` para `x` em TDZ também lança ReferenceError (diferente de `x` não declarado).
- Em classe derivada, `this` fica em TDZ até `super()` (BindThisValue).

### 4.4 Closures

- Função cria objeto com `[[Environment]]` = ambiente léxico no momento da criação.
- Ao chamar, o novo ambiente de função tem como pai o `[[Environment]]` (9.4.x [memória]).
- `for (let ...)` cria um ambiente novo por iteração (14.7.4.4, ver 3.3).

### 4.5 this, arguments e new.target

- **this** em função normal: `thisArgument` da chamada, ou `undefined`/objeto global conforme modo estrito (10.2.1.2 [memória]).
- **this** em arrow: vem do ambiente léxico (sem binding próprio).
- **arguments mapeado** (10.4.4 [confirmado seção]): só em função não estrita com parâmetros simples; índices do objeto `arguments` são ligados aos parâmetros (alterar um altera o outro) enquanto o índice existir.
- **arguments não mapeado**: estrito, com parâmetros não simples (default, rest, destructuring), ou com `arguments` como ... [memória]: cópia independente.
- **new.target**: ver 2.13.

### 4.6 Instanciações (9.4 e cláusulas 10, 14, 16 [memória dos números])

- **GlobalDeclarationInstantiation** (16.1.7 [memória]): valida conflitos de declarações globais (`let` contra `var` ou `function` já existentes), cria `var` e funções globais no objeto global, cria `let`/`const`/`class` no registro declarativo global em TDZ.
- **FunctionDeclarationInstantiation** (10.2.11 [memória]): cria ambiente da função, parâmetros (com `arguments` conforme 4.5), `var` e funções, em ordem; parâmetros com default usam ambiente separado quando há `eval` ou closures.
- **BlockDeclarationInstantiation** (14.2.3 [memória]): cria ambiente de bloco; `let`/`const`/`class` em TDZ; funções de bloco inicializadas.
- **eval direto**: cria `var` no ambiente da função de quem chamou; em estrito, ambiente próprio.

---

## 5. Funções, geradores e async (cláusulas 10.2, 15, 27)

### 5.1 Chamada de função comum (10.2.1 [memória])

**[[Call]](thisArgument, args)**
1. `callerContext` = contexto atual; cria contexto de execução novo.
2. Cria ambiente de função (4.6) com pai `[[Environment]]`.
3. Se `this` não é estrito: converte `thisArgument` (`undefined`/`null` vira objeto global; primitivo vira objeto).
4. `OrdinaryCallEvaluateBody`: executa o corpo; devolve o valor do `return` ou `undefined`.

### 5.2 Construtores e classes (10.2.2 [memória])

**[[Construct]](args, newTarget)**
- Função normal: cria objeto com protótipo `newTarget.prototype` (ou `%Object.prototype%`), chama o corpo com esse `this`; se o corpo retorna objeto, esse objeto vira o resultado.
- **Classe base**: igual, mas antes roda os campos de instância (`InitializeInstanceElements`).
- **Classe derivada**: `this` começa em TDZ (ambiente de função); `super(args)` (13.3.7.1 [memória]) faz `Construct(parent, args, newTarget)`, depois `BindThisValue` e roda os campos de instância; `this` antes de `super` lança ReferenceError. Se o construtor retorna valor não objeto, o resultado é `this` (que precisa estar inicializado).
- **Bound function** (10.4.1.2 [confirmado seção]): `[[Call]]` chama a função alvo com `boundThis` e args ligados; `[[Construct]]` chama `Construct` no alvo, com `newTarget` trocado se for a própria bound.

### 5.3 Geradores (27.5 [memória do número], 27.5.3.x)

- Chamar função `function*` não executa o corpo: cria objeto gerador com estado `suspendedStart` e um **quadro (frame) salvo**.
- O quadro guarda: contexto de execução (ambiente lexical atual, `this`, pilha de operandos, valores de temporários), a posição de instrução (ponto de `yield`), e a pilha de tratadores de `try`/`finally` ativos.
- `next(v)`: retoma o quadro; o `yield` anterior devolve `v`.
- `yield x`: suspende; devolve `{value: x, done: false}`.
- `return(v)`: força um completion `return` no ponto de suspensão; passa pelos `finally`.
- `throw(e)`: força um completion `throw` no ponto de suspensão; pode ser capturado por `try/catch` dentro do gerador.
- `yield*`: delega a outro iterável (repete `next`/`throw`/`return` do iterador interno).
- Estado `executing` impede chamada recursiva ao mesmo gerador (TypeError).

**Necessário guardar em um quadro**: registro de operandos, locais (incluindo o ambiente atual), ponto de instrução, pilha de tratadores de exceção, estado do `yield*` em andamento.

### 5.4 async e await (27.7 [memória], 6.2.3 e 9.5 [confirmado seção de jobs])

- Função `async`: chamada cria Promise de resultado e executa o corpo até o primeiro `await`.
- **Await(valor)**: converte com `PromiseResolve`; registra reações `onFulfilled`/`onRejected` que retomam o quadro com o valor ou com erro; suspende o quadro como o gerador.
- Quando o corpo termina: resolve (ou rejeita) a Promise de resultado.
- **Async generator** (27.6 [memória]): combina gerador e await; fila de requisições de `next`/`throw`/`return`; cada `yield` faz await do valor.

### 5.5 Jobs e fila de microtarefas (9.5 [confirmado seção])

- Job: função sem argumentos que roda a partir de uma fila. Fila de jobs de Promise (microtarefas) é esvaziada depois de cada script/tarefa.
- **HostEnqueuePromiseJob** (9.5.4 [memória]): operação do host para adicionar um job; no Falcão, o motor deve oferecer a fila de microtarefas ao host.
- `PromiseReactionJob` e `PromiseResolveThenableJob` (27.2.2 [memória]): são os jobs que resolvem reações e promessas "thenable".
- Ordem: todos os jobs enfileirados (inclusive os criados durante a execução de outros) rodam antes de qualquer tarefa do host.

---

## 6. Objetos: métodos internos e objetos exóticos (10.1 e 10.4)

### 6.1 Métodos internos ordinários (10.1 [confirmado seção])

| Método | Seção (memória) | Essência |
|---|---|---|
| [[GetPrototypeOf]] | 10.1.1 | devolve protótipo (ou null) |
| [[SetPrototypeOf]] | 10.1.2 | troca protótipo; sem ciclos; falso se não extensível |
| [[IsExtensible]] | 10.1.3 | flag de extensibilidade |
| [[PreventExtensions]] | 10.1.4 | desliga extensibilidade |
| [[GetOwnProperty]] | 10.1.5 | descritor próprio ou undefined |
| [[DefineOwnProperty]] | 10.1.6 | validação conforme ValidateAndApplyPropertyDescriptor |
| [[HasProperty]] | 10.1.7 | próprio ou na cadeia de protótipos |
| [[Get]] | 10.1.8 | próprio; se não tem, `[[Get]]` do protótipo, com receiver |
| [[Set]] | 10.1.9 | OrdinarySet: escreve no próprio ou com `[[DefineOwnProperty]]` no receiver |
| [[Delete]] | 10.1.10 | remove próprio configurável; falso senão |
| [[OwnPropertyKeys]] | 10.1.11 | ordem: índices inteiros ascendentes, depois strings na ordem de criação, depois símbolos na ordem de criação |

**Ordem de chaves** (10.1.11.1 [memória do número]): `OrdinaryOwnPropertyKeys` aplica exatamente essa ordem; `for-in` e `Object.keys` herdam dela (mas `for-in` percorre a cadeia).

**Descritor de propriedade** (6.2.6 [memória]): dados (`value`, `writable`) ou acessor (`get`, `set`); `enumerable` e `configurable` em ambos.

### 6.2 Objetos de função (10.2 e 10.3 [confirmado seções])

- 10.2: função ECMAScript (tem `[[Environment]]`, `[[ECMAScriptCode]]`, `[[ConstructorKind]]`, `[[HomeObject]]`, `[[Fields]]`).
- 10.3: função embutida (`[[Call]]` escrito em Rust no Falcão).
- 10.4.1: bound function.

### 6.3 Objetos exóticos (10.4 [confirmado seção])

| Exótico | Seção (memória) | Diferença |
|---|---|---|
| Array | 10.4.2 | `[[DefineOwnProperty]]` mantém `length` (ArraySetLength, 10.4.2.4) |
| String (objeto empacotador) | 10.4.3 | índices e `length` próprios e não graváveis |
| Arguments mapeado / não mapeado | 10.4.4 | ver 4.5 |
| Integer-indexed (TypedArray) | 10.4.5 | índices numéricos vão para o buffer; índice inválido é ignorado |
| Module namespace | 10.4.6 | propriedades somente leitura, ligadas aos módulos |
| Bound function | 10.4.1 | ver 5.2 |
| Proxy | 10.5 [confirmado seção] | cada método interno chama um trap, com invariantes |

### 6.4 Proxy (10.5 [confirmado seção])

- Cada método interno tem um trap (ex.: `get`, `set`, `has`, `deleteProperty`, `ownKeys`, `getOwnPropertyDescriptor`, `defineProperty`, `getPrototypeOf`, `setPrototypeOf`, `isExtensible`, `preventExtensions`, `apply`, `construct`).
- Invariantes: o resultado do trap é conferido contra o alvo; violação gera TypeError.
- Proxy revogado: qualquer operação gera TypeError.

---

## 7. Objetos embutidos (cláusulas 19 a 28)

Lista de clausulas e subtipos [confirmada pelo sumário, quando marcada; restante de memória]:

| Cláusula | Conteúdo | Confirmado |
|---|---|---|
| 19 | Propriedades globais (`globalThis`, `NaN`, `Infinity`, `undefined`, `eval`, `isFinite`, `isNaN`, `parseFloat`, `parseInt`, `decodeURI`, `encodeURI`, etc.) | 19.2.6.1 `decodeURI` [confirmado] |
| 20 | Objetos fundamentais: `Object` (20.1), `Function` (20.2), `Boolean` (20.3), `Symbol` (20.4), `Error` e subtipos (20.5) | 20.4.1 Symbol, 20.5.5.4 SyntaxError, 20.5.5.5 TypeError, 20.5.5.6 URIError, 20.5.7.1 AggregateError, 20.5.8.1 SuppressedError [confirmados] |
| 21 | Números e datas: `Number` (21.1), `BigInt` (21.2), `Math` (21.3), `Date` (21.4) | 21.3.1.9 Math [confirmado] |
| 22 | Processamento de texto: `String` (22.1), `RegExp` (22.2) | 22.1.1 construtor String, 22.1.5 String Iterator [confirmados] |
| 23 | Coleções indexadas: `Array` (23.1), `TypedArray` (23.2) | 23.1.3.27 Array.prototype.shift, 23.2.1 %TypedArray% [confirmados] |
| 24 | Coleções com chave: `Map` (24.1), `Set` (24.2), `WeakMap` (24.3), `WeakSet` (24.4) | 24.3.1 WeakMap, 24.4.1 WeakSet [confirmados] |
| 25 | Dados estruturados: `ArrayBuffer` (25.1), `SharedArrayBuffer` (25.2), `DataView` (25.3), `Atomics` (25.4), `JSON` (25.5) | 25.2.3 SharedArrayBuffer [confirmado] |
| 26 | Abstrações de controle: `WeakRef` (26.1), `FinalizationRegistry` (26.2) | 26.1.1 WeakRef [confirmado] |
| 27 | Iteradores, geradores, Promise, funções assíncronas | 27.1.3.2.2.1 Iterator.from [confirmado]; Promise, geradores (27.5), async (27.7) [memória] |
| 28 | Reflexão: `Proxy` (28.1), `Reflect` (28.2) | [memória] |

**Contagens de métodos e propriedades por objeto: NÃO levantadas nesta sessão.** Para não inventar números, o levantamento deve ser feito a partir do texto das seções 19 a 28 (ou da lista de propriedades no MDN) e colado em tabela própria antes de ser usado para priorizar a implementação.

Objetos embutidos citados no pedido, com a cláusula onde a norma os define (cláusula [memória] salvo nota):
- Object (20.1), Function (20.2), Boolean (20.3), Symbol (20.4), Error e subtipos (20.5), Number (21.1), BigInt (21.2), Math (21.3), Date (21.4), String (22.1), RegExp (22.2), Array (23.1), TypedArrays (23.2), Map (24.1), Set (24.2), WeakMap (24.3), WeakSet (24.4), WeakRef (26.1), FinalizationRegistry (26.2), ArrayBuffer (25.1), SharedArrayBuffer (25.2), DataView (25.3), Atomics (25.4), JSON (25.5), Iterator helpers (27.1), Promise (27.2), Proxy (28.1), Reflect (28.2), geradores (27.5), AsyncFunction (27.7), AsyncGenerator (27.6).

---

## 8. Sugestão de famílias de instruções para o bytecode do Falcão

Sugestão nossa, desenhada a partir das seções acima. Não é cópia de conjunto de nenhum motor. Cada família diz qual item da norma atende.

### 8.1 Constantes e pilha

- **Carregar constante** (`CARREGAR_CONST idx`): número, string, BigInt, booleano, `null`, `undefined`. Atende 13.2 (literais), 6.2.6.
- **Empilhar/desempilhar** (`EMPILHAR`, `DESCARTAR`, `DUPLICAR`, `TROCAR`): pilha de operandos; atende 13.15 (valor de atribuição), 13.16 (vírgula).

### 8.2 Variáveis e ambientes

- **Ler variável** (`LER_NOME nome`): `GetValue` de referência de identificador, com erro de não resolvido (6.2.5.5, 9.1). Nome resolvido em tempo de compilação quando possível (locais e closures por índice).
- **Gravar variável** (`GRAVAR_NOME nome`): `PutValue` (6.2.5.6), respeitando estrito.
- **Ler/gravar local por índice** (`LER_LOCAL i`, `GRAVAR_LOCAL i`): para escopo de função sem `eval`/`with` (otimização segura).
- **Ler/gravar em closure** (`LER_CLOSURE profundidade, i`): depende de 4.4.
- **Verificar TDZ** (`LER_LOCAL_TDZ`): ReferenceError para `let`/`const`/`class` não inicializados (4.3).
- **Inicializar binding** (`INICIALIZAR_BINDING`): fim da TDZ (14.3.1.2).
- **Criar ambiente** (`ENTRAR_AMBIENTE tipo`, `SAIR_AMBIENTE`): bloco, catch, `for (let)`, `with`, função (4.1, 14.2.3).
- **Copiar ambiente por iteração** (`NOVA_ITERACAO_LET`): 14.7.4.4.
- **Declarar global** (`DECLARAR_VAR_GLOBAL`, `DECLARAR_FUNCAO_GLOBAL`, `DECLARAR_LEXICO_GLOBAL`): 16.1.7.
- **Remover (delete)** (`REMOVER_NOME`, `REMOVER_PROP`): 13.5 (delete).

### 8.3 Propriedades e objetos

- **Ler propriedade** (`LER_PROP`, `LER_PROP_COMPUTADA`): `[[Get]]` com `ToPropertyKey` quando computada, e `ToObject` (13.3.2, 7.3.2, 10.1.8).
- **Gravar propriedade** (`GRAVAR_PROP`, `GRAVAR_PROP_COMPUTADA`): `[[Set]]` (7.3.4, 10.1.9).
- **Remover propriedade** (`REMOVER_PROP`): `[[Delete]]` (13.5, 7.3.9).
- **Testar presença** (`TEM_PROP`): operador `in` (13.10, 7.3.11).
- **Testar instância** (`INSTANCEOF`): 13.10.2, 7.3.21.
- **Criar objeto/array** (`NOVO_OBJETO`, `NOVO_ARRAY`, `DEFINIR_CAMPO_DADOS`, `DEFINIR_CAMPO_COMPUTADO`, `DEFINIR_ACESSOR`, `DEFINIR_METODO`): 13.2.4, 13.2.5, 7.3.5, 7.3.8.
- **Espalhar** (`ESPALHAR_OBJETO`, `ESPALHAR_ITER`): spread de objeto (CopyDataProperties) e iterável em array/chamada (13.2.4, 13.3.8).
- **Definir protótipo literal** (`DEFINIR_PROTOTIPO`): `__proto__:` em literal (13.2.5.5).

### 8.4 Operadores

- **Aritmética** (`SOMAR`, `SUBTRAIR`, `MULTIPLICAR`, `DIVIDIR`, `RESTO`, `EXPOENTE`): com ToNumeric e ramo BigInt (13.6 a 13.8, 7.1.3).
- **Bit a bit e deslocamento** (`E_BIT`, `OU_BIT`, `XOR_BIT`, `DESLOC_ESQ`, `DESLOC_DIR`, `DESLOC_DIR_SEM_SINAL`, `NEGAR_BIT`): 13.9, 13.12, 7.1.6, 7.1.7.
- **Unários** (`NEGAR`, `MAIS_UNARIO`, `NAO_LOGICO`, `TIPO_DE`, `VAZIO`, `INCREMENTAR`, `DECREMENTAR`): 13.4, 13.5.
- **Comparação** (`MENOR`, `MENOR_IGUAL`, `MAIOR`, `MAIOR_IGUAL`): IsLessThan (7.2.12), 13.10.
- **Igualdade** (`IGUAL_ESTRITO`, `IGUAL_SOLTO`, e as negações como instruções ou como `NEGAR` depois): 7.2.13, 7.2.14, 13.11.
- **Conversões** (`PARA_NUMERO`, `PARA_NUMERICO`, `PARA_TEXTO`, `PARA_BOOL`, `PARA_CHAVE`, `PARA_OBJETO`, `PARA_PRIMITIVO dica`): 7.1.x.
- **Concatenação de template** (`CONCATENAR_TEXTO n`): 13.2.8.

### 8.5 Atribuições e referências

- **Referência** (`REF_PROP`, `REF_NOME`, `REF_LOCAL`): cria referência sem ler, para atribuições compostas e `++`/`--` avaliarem a base só uma vez (6.2.5, 13.15).
- **Atribuição composta** (`OPERAR_REF op`): combina referência e operador.
- **Curto-circuito de atribuição lógica** (`SE_CURTO_CIRCUITO_LOGICO tipo, alvo`): `&&=`, `||=`, `??=` (13.15.2).

### 8.6 Controle de fluxo

- **Saltos** (`SALTAR alvo`, `SALTAR_SE_FALSO alvo`, `SALTAR_SE_VERDADEIRO alvo`, `SALTAR_SE_NULO_OU_INDEFINIDO alvo`): if, laços, ternário, `&&`, `||`, `??`, `?.` (14.6, 14.7, 13.14, 13.13, 13.3.9).
- **Saltar para tarefa de curto-circuito** (`SALTAR_SE_NULO_OU_INDEFINIDO_E_LIMPAR alvo`): encadeamento opcional (13.3.9).
- **Tabela de saltos de switch** (`SWITCH_COMPARA idx alvo`): igualdade estrita, com fall-through (14.12).
- **Rótulos e completions** (`ENTRAR_ROTULO`, `SAIR_ROTULO`): labelled statement (14.13).

### 8.7 Chamadas e funções

- **Chamar** (`CHAMAR n`, `CHAMAR_METODO n`, `CHAMAR_COM_ESPALHAMENTO`): Call (7.3.13), this de membro (13.3.6), chamada de `with` (3.6).
- **Chamar eval direto** (`CHAMAR_EVAL_DIRETO n`): só quando o identificador resolve para `%eval%` (13.3.6.1, 19.2.1).
- **Construir** (`NOVO n`): Construct (7.3.14, 13.3.5).
- **Chamar super** (`CHAMAR_SUPER n`): 13.3.7.1, BindThisValue.
- **Ler super** (`LER_SUPER_PROP`, `GRAVAR_SUPER_PROP`): 13.3.7.
- **Criar closure** (`CRIAR_FUNCAO indice_do_modelo`): objeto de função com [[Environment]] atual (10.2, 13.2.6).
- **Criar arrow / método / classe** (`CRIAR_ARROW`, `CRIAR_METODO`, `CRIAR_CLASSE`): 13.2.6, 15.7.
- **Retornar** (`RETORNAR`): completion return (14.10).
- **Lançar** (`LANCAR`): completion throw (14.14).
- **Novo.target e this** (`LER_NEW_TARGET`, `LER_THIS`, `INICIALIZAR_THIS`): 13.3.12, 13.2.1.1, 10.2.2.

### 8.8 Exceções e finally

- **Entrar em try** (`ENTRAR_TRY alvo_catch, alvo_finally`): empilha tratador (14.15).
- **Sair de try** (`SAIR_TRY`): desempilha tratador.
- **Entrar em finally** (`ENTRAR_FINALLY`): guarda completion pendente (normal, return, break, continue ou throw) (14.15.3).
- **Fim de finally** (`FIM_FINALLY`): restaura completion pendente e continua, ou executa o salto/return/throw pendente.
- **Capturar valor** (`EMPILHAR_EXCECAO`): valor do throw dentro do catch (14.15.2).

### 8.9 Iteração

- **Obter iterador** (`OBTER_ITERADOR sync|async`): 7.4.4.
- **Passo de iterador** (`PASSO_ITERADOR alvo_fim`): 7.4.9; se `done`, salta.
- **Fechar iterador** (`FECHAR_ITERADOR`, `FECHAR_ITERADOR_ASYNC`): 7.4.11; usado em `break`, `return`, `throw` saindo de `for-of`/destructuring (14.7.5.7).
- **Enumerar chaves** (`ENUMERAR_CHAVES`, `PASSO_CHAVES alvo_fim`): for-in (14.7.5.9, EnumerateObjectProperties).
- **Destructuring de array** (usa `OBTER_ITERADOR` e `PASSO_ITERADOR`): 14.3.3 e 8.6.

### 8.10 Geradores e async

- **Criar gerador** (`CRIAR_GERADOR`): função `function*` gera objeto com quadro inicial (27.5).
- **Suspender** (`YIELD`): salva quadro (ver 5.3) e devolve `{value, done}`.
- **Retomar** (`RETOMAR_GERADOR modo`): modo `next`, `throw` ou `return` (27.5.3).
- **Delegar** (`YIELD_ESTRELA`): 14.4.14 [memória].
- **Await** (`AWAIT`): suspende, registra reações de Promise (6.2.3, 27.7).
- **Criar Promise de async** (`INICIAR_ASYNC`): cria promessa de resultado (27.7.5).
- **Enfileirar microtarefa** (host): `ENFILEIRAR_JOB`: 9.5.

### 8.11 Escopo e classes

- **Campo privado** (`LER_PRIVADO nome_privado`, `GRAVAR_PRIVADO nome_privado`, `TEM_PRIVADO nome_privado`): 13.3.2.1, 9.2.
- **Inicializar campos de instância** (`INICIALIZAR_CAMPOS`): 15.7 [memória].
- **Criar protótipo/método** (`DEFINIR_METODO_CLASSE`): 15.7.

### 8.12 Template e regex

- **Objeto de template cacheado** (`OBTER_TEMPLATE idx_site`): um por site (13.2.8.4).
- **Criar regex** (`CRIAR_REGEX padrao, flags`): 13.2.7.

### 8.13 Diagnóstico

- **Marcar linha** (`LINHA n`): para pilha de erros.
- **Depurar** (`DEBUGGER`): sem efeito obrigatório (14.x).

---

## 9. Pendências antes de implementar

1. Ler, no texto oficial de tc39.es, cada algoritmo marcado [memória] e copiar o número exato de seção (a ferramenta usada não trouxe o corpo).
2. Levantar contagens de métodos (seção 7) a partir das seções 19 a 28.
3. Confirmar a versão ("ECMAScript 2027" no sumário) que o Falcão adota como alvo, e marcar quais recursos novos (Iterator helpers, Set methods, Array grouping) entram na primeira fase.
4. Decidir como tratar `eval` direto e `with` (eles exigem ambiente dinâmico; afetam a otimização de locais por índice, ver 8.2).
