var a = Array.from({ length: 1e6 }, (_, i) => i);
var r = 0; for (let k = 0; k < 5; k++) r += a.map(x => x * 2).filter(x => x % 3 === 0).reduce((s, x) => s + x, 0);
