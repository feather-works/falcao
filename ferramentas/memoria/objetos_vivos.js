// 1 milhão de objetos pequenos vivos ao mesmo tempo: bytes por objeto.
var lista = [];
for (var i = 0; i < 1e6; i++) lista.push({ x: i, y: i + 1 });
var r = lista.length;
