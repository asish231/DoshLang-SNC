// Every runnable snippet, imported as raw text. Each file is compile+run
// verified by scripts/verify-snippets.mjs — never add one without verifying.
import hello from '../../snippets/hello.sn?raw';
import printing from '../../snippets/printing.sn?raw';
import vars from '../../snippets/vars.sn?raw';
import ops from '../../snippets/ops.sn?raw';
import casting from '../../snippets/casting.sn?raw';
import ifelse from '../../snippets/ifelse.sn?raw';
import matchdemo from '../../snippets/matchdemo.sn?raw';
import loops from '../../snippets/loops.sn?raw';
import forin from '../../snippets/forin.sn?raw';
import funcs from '../../snippets/funcs.sn?raw';
import defaults from '../../snippets/defaults.sn?raw';
import multiret from '../../snippets/multiret.sn?raw';
import strings from '../../snippets/strings.sn?raw';
import lists from '../../snippets/lists.sn?raw';
import maps from '../../snippets/maps.sn?raw';
import nullops from '../../snippets/nullops.sn?raw';
import errors from '../../snippets/errors.sn?raw';
import trydemo from '../../snippets/trydemo.sn?raw';
import mathuse from '../../snippets/mathuse.sn?raw';
import point from '../../snippets/point.sn?raw';
import inherit from '../../snippets/inherit.sn?raw';
import poly from '../../snippets/poly.sn?raw';
import chansimple from '../../snippets/chansimple.sn?raw';
import selectdemo from '../../snippets/selectdemo.sn?raw';
import jsondemo from '../../snippets/jsondemo.sn?raw';
import filedemo from '../../snippets/filedemo.sn?raw';
import dsa_kadane from '../../snippets/dsa_kadane.sn?raw';
import dsa_binsearch from '../../snippets/dsa_binsearch.sn?raw';
import dsa_twosum from '../../snippets/dsa_twosum.sn?raw';
import dsa_quicksort from '../../snippets/dsa_quicksort.sn?raw';
import dsa_lru from '../../snippets/dsa_lru.sn?raw';
import dsa_workers from '../../snippets/dsa_workers.sn?raw';

export const SNIPPETS = {
  hello,
  printing,
  vars,
  ops,
  casting,
  ifelse,
  matchdemo,
  loops,
  forin,
  funcs,
  defaults,
  multiret,
  strings,
  lists,
  maps,
  nullops,
  errors,
  trydemo,
  mathuse,
  point,
  inherit,
  poly,
  chansimple,
  selectdemo,
  jsondemo,
  filedemo,
  dsa_kadane,
  dsa_binsearch,
  dsa_twosum,
  dsa_quicksort,
  dsa_lru,
  dsa_workers,
};

// Captured `stdout` of each snippet (run against ./snc). Workers intentionally
// omitted: two threads race, so line order varies run to run.
export const OUTPUTS = {
  hello: 'Hello, World!',
  printing: 'plain text output\n42\nhello Ada\n2 + 2 = 4\nname: Ada!',
  vars: 'Ada\n10\ntrue\n15\n68',
  ops: '13\n20\n3\n2\ntrue\nfalse\nfalse',
  casting: 'n as text: 42\n7 * 6 = 42\nok = true',
  ifelse: 'A\nB\nF',
  matchdemo: 'User is active\ntwo',
  loops: '0\n1\n2\n3\n4\n3\n2\n1\n1\n2\n4',
  forin: 'Apple\nMango\nBanana\nBob is 30\nAnn is 25',
  funcs: 'Hello, Ada\n10\n13',
  defaults: 'Guest\nGuest\nAce\nAce\nNeo',
  multiret: '5\ndivision failed',
  strings: '11\nHello\ntrue\nHELLO WORLD\nhello world\nHello SNlang\nHello\nWorld\nhi Ada',
  lists: '3\n10\n99\n4\ntrue\nfalse',
  maps: '30\n26\n2\n1',
  nullops: 'anon\nAda\nno lucky number\n7\n8',
  errors: '5\nok',
  trydemo: '5\ndivision by zero',
  mathuse: '5\n9\n3\n10\n256',
  point: '30\n10',
  inherit: 'Canine\nWoof\nLab',
  poly: 'woof',
  chansimple: '42',
  selectdemo: '1\n77',
  jsondemo: '7\nhi\n[7,"hi"]\ntime\n1970-01-01T00:00:00Z',
  filedemo: 'hello file io',
  dsa_kadane: 'Max Subarray Sum: 6',
  dsa_binsearch: 'Target 23 found at index: 5',
  dsa_twosum: 'Indices: 0 and 1',
  dsa_quicksort: 'Sorted: 7 items\n4\n4\n8\n19\n27\n33\n51',
  dsa_lru: '1\n2\n3',
};

export const PRESETS = [
  { id: 'hello', title: 'Hello, World!' },
  { id: 'printing', title: 'Printing & interpolation' },
  { id: 'funcs', title: 'Functions' },
  { id: 'lists', title: 'Lists' },
  { id: 'maps', title: 'Maps' },
  { id: 'chansimple', title: 'Channel message' },
  { id: 'dsa_kadane', title: "Kadane's algorithm" },
  { id: 'dsa_quicksort', title: 'Quicksort' },
  { id: 'poly', title: 'Blueprint polymorphism' },
  { id: 'jsondemo', title: 'JSON + time' },
];
