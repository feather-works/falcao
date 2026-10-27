// Map com 500 mil entradas texto → objeto, como um índice em memória.
var m = new Map();
for (var i = 0; i < 5e5; i++) m.set('chave' + i, { id: i, ativo: true });
var r = m.size;
