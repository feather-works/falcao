function f() { let s = ''; for (let i = 0; i < 3e5; i++) { s += 'item' + i + ';'; } let n = 0; for (const p of s.split(';')) n += p.length; return n + s.indexOf('item299999') + s.toUpperCase().length; }
var r = f();
