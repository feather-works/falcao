var m = new Map(); for (let i = 0; i < 5e5; i++) m.set('k' + i, i);
var r = 0; for (let k = 0; k < 4; k++) for (let i = 0; i < 5e5; i++) r += m.get('k' + i);
var st = new Set(); for (let i = 0; i < 1e6; i++) st.add(i % 1000);
