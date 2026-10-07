// The complete SNlang guide: zero to productive. Every `snippet` id refers to
// src/data/snippets.js and every program is verified by
// scripts/verify-snippets.mjs. `output` is the exact captured stdout.
export const CHAPTERS = [
  {
    id: 'basics',
    num: '01',
    title: 'Your first program',
    tagline: 'What an SNlang program looks like, and how to print things.',
    sections: [
      {
        h: 'Anatomy of a program',
        body: `Every SNlang program starts at a function called \`main\`. Functions are declared with \`fn\`, take their parameters in parentheses, and wrap their body in curly braces \`{\` \`}\`.

- \`fn main() {\` ... \`}\` is the entry point; the program runs top to bottom inside it.
- Comments start with \`//\` and run to the end of the line.
- Statements do not need semicolons.`,
        snippet: 'hello',
        output: 'hello',
      },
      {
        h: 'Printing values',
        body: `\`print\` writes a value followed by a newline. It accepts text, numbers, and booleans.

- \`"hello {name}"\` is string interpolation: anything inside \`{\` \`}\` is evaluated and pasted into the text.
- \`+\` joins (concatenates) two strings.
- \`printn\` is the variant that does not add a trailing newline.`,
        snippet: 'printing',
        output: 'printing',
      },
    ],
  },
  {
    id: 'variables',
    num: '02',
    title: 'Variables',
    tagline: 'Declaring, reading, and updating typed variables.',
    sections: [
      {
        h: 'Declaring variables',
        body: `Variables are declared with a type, a name, and a starting value. The type is checked at compile time and never changes.

- \`int\` for whole numbers, \`str\` for text, \`bool\` for \`true\`/\`false\`.
- Reassign later with \`=\`. The new value must have the same type.
- Names use letters, digits, and underscores, and cannot start with a digit.`,
        snippet: 'vars',
        output: 'vars',
      },
      {
        h: 'Compound assignment',
        body: `The end of the \`vars\` program uses shortcuts you will see everywhere: \`+=\`, \`-=\`, and \`*=\` update a variable in place.

\`score += 5\` means exactly \`score = score + 5\`. They only work on numbers, and the result keeps the same type.`,
      },
    ],
  },
  {
    id: 'datatypes',
    num: '03',
    title: 'Numbers, logic, and casting',
    tagline: 'Arithmetic, comparisons, English-word logic, and converting types.',
    sections: [
      {
        h: 'Arithmetic and comparison',
        body: `\`int\` arithmetic behaves like C: \`/\` on two integers divides and truncates, \`%\` gives the remainder, and parentheses control grouping.

Comparisons (\`==\`, \`!=\`, \`<\`, \`<=\`, \`>\`, \`>=\`) produce a \`bool\`. Logic uses English words instead of symbols: \`and\`, \`or\`, \`not\`.`,
        snippet: 'ops',
        output: 'ops',
      },
      {
        h: 'Converting with cast',
        body: `SNlang never converts types silently. \`cast(value, str)\` renders a value as text, which is how you mix numbers into messages.

- \`cast(42, str)\` gives \`"42"\`.
- Interpolation does this automatically: \`"7 * 6 = {7 * 6}"\` evaluates the expression and pastes the result.`,
        snippet: 'casting',
        output: 'casting',
      },
    ],
  },
  {
    id: 'control',
    num: '04',
    title: 'Control flow',
    tagline: 'Making decisions with if and match.',
    sections: [
      {
        h: 'if / else if / else',
        body: `Branches test a \`bool\` condition. Parentheses around the condition are required, and each branch is a \`{ ... }\` block.

- Chain alternatives with \`else if\` and finish with \`else\`.
- A function can \`return\` early from any branch, which keeps grading-style ladders flat and readable.`,
        snippet: 'ifelse',
        output: 'ifelse',
      },
      {
        h: 'match: multi-way branches',
        body: `\`match\` compares one value against many patterns. It works on strings, integers, and enum values (chapter 12 shows enums).

- Each arm is a value followed by a block. No \`break\` is needed: only the matched arm runs.
- \`default\` catches everything else. Leave it out and an unmatched value simply does nothing.`,
        snippet: 'matchdemo',
        output: 'matchdemo',
      },
    ],
  },
  {
    id: 'loops',
    num: '05',
    title: 'Loops',
    tagline: 'Counted loops, while loops, iterating collections, stop and skip.',
    sections: [
      {
        h: 'Counted for and while',
        body: `The counted loop packs start, condition, and step into one line, separated by commas: \`for (int i = 0, i < 5, i += 1)\`.

\`while\` repeats a block as long as its condition holds. Both use \`{\` \`}\` blocks, like everything else in the language.`,
        snippet: 'loops',
        output: 'loops',
      },
      {
        h: 'stop and skip',
        body: `The tail of the \`loops\` program shows loop control. It prints \`1 2 4\`:

- \`skip\` jumps to the next iteration (other languages call this \`continue\`).
- \`stop\` exits the loop immediately (other languages call this \`break\`).`,
      },
      {
        h: 'for-in over collections',
        body: `The same \`for\` keyword with \`in\` walks a collection instead of counting. It works on lists, on map keys via \`.keys()\`, and on strings.

- \`for (fruit in fruits)\` binds each element in turn.
- The loop variable is a fresh copy each round; mutating it does not change the collection.`,
        snippet: 'forin',
        output: 'forin',
      },
    ],
  },
  {
    id: 'functions',
    num: '06',
    title: 'Functions',
    tagline: 'Declaring functions, default arguments, and returning many values.',
    sections: [
      {
        h: 'Declaring and calling',
        body: `\`fn name(params) -> ReturnType { ... }\` declares a function. Every parameter is typed, and the return type after \`->\` may be omitted for functions that return nothing.

- Call with \`name(args)\`. Arguments are checked against parameter types.
- Functions can be declared before or after their callers in the same file.`,
        snippet: 'funcs',
        output: 'funcs',
      },
      {
        h: 'Default parameters',
        body: `Parameters can carry defaults (\`str name = "Guest"\`). Callers may omit any trailing arguments, and each call fills in what was skipped.

Defaults are evaluated at each call, left to right, so \`greet("Neo", 1)\` overrides both while \`greet()\` uses both defaults.`,
        snippet: 'defaults',
        output: 'defaults',
      },
      {
        h: 'Returning multiple values',
        body: `A function can return a tuple such as \`(int, bool)\`, and the caller unpacks it in one declaration: \`int q, ok = divide(10, 2)\`.

This is the idiom behind error handling too: the next chapters use \`(T, error)\` tuples pervasively, so getting comfortable with unpacking now pays off immediately.`,
        snippet: 'multiret',
        output: 'multiret',
      },
    ],
  },
  {
    id: 'strings',
    num: '07',
    title: 'Strings',
    tagline: 'Text methods, interpolation, and searching.',
    sections: [
      {
        h: 'String methods',
        body: `Strings carry their own toolbox, called with dot syntax. The program below exercises the whole set, and prints each result so you can see exactly what every method returns.

- \`length()\`, \`slice(start, count)\`, \`contains(sub)\`
- \`upper()\`, \`lower()\`, \`replace(old, new)\`
- \`split(sep)\` returns a \`list<str>\`
- \`"hi {name}"\` interpolates any expression, and \`"x" in s\` tests for a substring.`,
        snippet: 'strings',
        output: 'strings',
      },
    ],
  },
  {
    id: 'collections',
    num: '08',
    title: 'Lists and maps',
    tagline: 'Ordered sequences and key-value lookup.',
    sections: [
      {
        h: 'Lists',
        body: `\`list<int>\` is an ordered, growable sequence. Literals use square brackets: \`[10, 20, 30]\`.

- Index from \`0\`; \`nums[1] = 99\` replaces an element in place.
- \`push(v)\` appends, \`length()\` counts, and \`v in nums\` tests membership.
- Lists of any type work the same way: \`list<str>\`, \`list<bool>\`, even nested \`list<list<int>>\`.`,
        snippet: 'lists',
        output: 'lists',
      },
      {
        h: 'Maps',
        body: `\`map<str, int>\` associates keys with values. Literals use braces: \`{"Bob": 30}\`.

- \`ages["Bob"]\` reads, \`ages["Ann"] = 26\` writes (adding the key if it is new).
- \`length()\` counts entries, \`keys()\` lists them for \`for-in\` loops, and \`contains(k)\` tests for a key without reading it.`,
        snippet: 'maps',
        output: 'maps',
      },
    ],
  },
  {
    id: 'nullsafe',
    num: '09',
    title: 'Null safety',
    tagline: 'Opt-in missing values with T?, none, and otherwise.',
    sections: [
      {
        h: 'Missing values are opt-in',
        body: `A plain \`str\` can never be missing. To allow absence, add \`?\`: \`str? nickname\`. The only extra value it can hold is \`none\`.

- \`otherwise\` supplies a fallback: \`nick otherwise "anon"\` evaluates to the name, or \`"anon"\` when it is \`none\`.
- \`if (lucky != none)\` narrows the type inside the branch: the compiler knows the value is present there.
- \`lucky!!\` asserts presence and unwraps. If you are wrong the program stops with a clear error, so prefer \`otherwise\` or an explicit check.`,
        snippet: 'nullops',
        output: 'nullops',
      },
    ],
  },
  {
    id: 'errors',
    num: '10',
    title: 'Error handling',
    tagline: 'Recoverable errors as values: (T, error), try, and panic.',
    sections: [
      {
        h: 'Errors are return values',
        body: `Fallible functions return a tuple: the value plus an \`error\`. Success carries \`none\`; failure carries \`error("message")\`.

- The caller unpacks both: \`int q, err = divide(10, 0)\`.
- \`err != none\` means something went wrong; \`err.message()\` explains what.
- There are no exceptions and no try/catch. An error you ignore stays visible in the type, right in the signature.`,
        snippet: 'errors',
        output: 'errors',
      },
      {
        h: 'try: early return on error',
        body: `\`try expr\` unwraps a tuple call: on success it yields the value, on error the current function returns that error to its own caller immediately.

Use \`try\` when there is nothing useful to do locally. Handle the tuple manually when you can recover, retry, or add context. For bugs that should never happen, \`panic("reason")\` stops the program.`,
        snippet: 'trydemo',
        output: 'trydemo',
      },
    ],
  },
  {
    id: 'modules',
    num: '11',
    title: 'Modules and the standard library',
    tagline: 'Reusing code with use: stdlib packages and your own files.',
    sections: [
      {
        h: 'Using the standard library',
        body: `\`use std.math\` imports a package, after which its functions are called directly: \`abs\`, \`max\`, \`min\`, \`pow\`, \`clamp\`, \`sign\`.

The lookup order is: the importing file's directory, then \`packages/\`, then \`stdlib/\`, then path dependencies from \`sn.toml\`. The Stdlib page of this site lists every package.`,
        snippet: 'mathuse',
        output: 'mathuse',
      },
      {
        h: 'Your own modules',
        body: `Any \`.sn\` file is a module. If \`calc.sn\` defines \`fn b()\`, another file in the same project calls it after \`use mylib.calc\` (dotted path, no extension, no quotes).

Packages add a manifest: \`snc pkg init --name myapp\` creates \`sn.toml\`, \`snc pkg add\` records dependencies, and \`snc pkg build\` compiles with them resolved. See the Tooling page for the full workflow.`,
      },
    ],
  },
  {
    id: 'blueprints',
    num: '12',
    title: 'Blueprints (objects)',
    tagline: 'Structured data with methods: construction, inheritance, polymorphism.',
    sections: [
      {
        h: 'Declaring and constructing',
        body: `A \`blueprint\` groups fields with the methods that operate on them. Inside a method, \`self\` is the current instance.

- Objects are built with \`new\`: \`new Point p(x: 10, y: 20)\` names each field.
- Methods are called with dots: \`p.sum()\`.
- Methods without \`self\` do not exist: every method sees its own object.

One gotcha for later: \`map\` and \`list\` fields start unset, so initialize them right after \`new\` (\`cache.store = {}\`) before calling methods that touch them.`,
        snippet: 'point',
        output: 'point',
      },
      {
        h: 'Inheritance with from',
        body: `\`blueprint Dog from Animal\` inherits every field and method of \`Animal\`. The subclass constructor sets parent fields too: \`new Dog d(species: "Canine", breed: "Lab")\`.

Inherited methods keep working through the subclass, and the subclass adds its own state and behavior on top.`,
        snippet: 'inherit',
        output: 'inherit',
      },
      {
        h: 'Polymorphism and contracts',
        body: `A \`list<Animal>\` accepts any subclass instance, and calling an overridden method dispatches to the real object at runtime.

Beyond that, blueprints support \`contract\` interfaces, \`closed\`/\`guarded\` access control, \`static\` methods called as \`Counter.create()\`, \`abstract\` methods subclasses must implement, and generics such as \`Box<T>\` with \`in\`/\`out\` variance. The language tour in \`docs/LANGUAGE.md\` details each one.`,
        snippet: 'poly',
        output: 'poly',
      },
    ],
  },
  {
    id: 'concurrency',
    num: '13',
    title: 'Concurrency',
    tagline: 'Threads as blocks, typed channels, and select.',
    sections: [
      {
        h: 'spawn and goroutine blocks',
        body: `Concurrency is written as blocks, not function launches. \`spawn { ... }\` runs an OS thread; \`goroutine { ... }\` runs on the shared scheduler.

- The block runs alongside the code after it. The main thread below keeps going while the block executes.
- Blocks share the enclosing scope, so give variables distinct names in sibling blocks.

There is no \`go\` keyword and no \`while (true)\` polling: threads coordinate through channels.`,
        snippet: 'chansimple',
        output: 'chansimple',
      },
      {
        h: 'Channels: send and receive',
        body: `Declare a channel with \`chan<int> ch\`. One side calls \`ch.send(value)\`, the other calls \`ch.receive()\`, and the two rendezvous: each waits for the other.

- \`ch.close()\` signals that no more values are coming.
- \`select(a, b, ms)\` waits on two channels at once with a millisecond timeout and returns which one is ready, or \`-1\`.
- The \`lock\` type guards shared memory when threads must touch the same data.`,
        snippet: 'selectdemo',
        output: 'selectdemo',
      },
    ],
  },
  {
    id: 'io',
    num: '14',
    title: 'Files, JSON, and time',
    tagline: 'Talking to the outside world with the standard library.',
    sections: [
      {
        h: 'Reading and writing files',
        body: `The globals \`file_write(path, text)\` and \`file_read(path)\` cover the common case in two lines. For more control, \`use std.file\` adds \`append\`, \`exists\`, \`delete\`, \`copy\`, \`move\`, \`mkdir\`, \`rmdir\`, and \`list_dir\`, all returning \`error\` values in the usual tuple style.

The program below writes a file and reads it back. It is shown statically because running it creates a file: try it locally with \`./snc\`. Press Run on this site only for programs without side effects.`,
        snippet: 'filedemo',
        runnable: false,
      },
      {
        h: 'JSON and clocks',
        body: `\`use std.json\` parses text into a \`json\` value and encodes it back: \`parse\` returns a \`(json, error)\` tuple, \`encode\` renders a string.

\`use std.time\` gives millisecond clocks (\`now_ms\`), sleeps (\`sleep_ms\`), and formatting (\`format\`). \`use std.os\` adds environment variables (\`getenv\`), arguments (\`args\`), and subprocesses (\`exec\`, \`system\`).`,
        snippet: 'jsondemo',
        output: 'jsondemo',
      },
    ],
  },
];
