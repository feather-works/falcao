var t = ''; for (let i = 0; i < 2e4; i++) t += 'user' + i + '@exemplo.com, ';
var r = 0; for (let k = 0; k < 20; k++) { r += t.match(/[a-z0-9]+@[a-z]+\.com/g).length; r += t.replace(/\d+/g, '#').length; }
