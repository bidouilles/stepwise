# Stepwise tracer: runs a student's program under sys.settrace and records a
# snapshot of the stack, globals and heap at every step.
#
# Input  (stdin):  JSON {"code": str, "stdin": str, "max_steps": int}
# Output (stdout): MARKER followed by JSON {"steps": [...], "stdout": str, ...}
import sys, json, io, os, re, time, types, inspect, builtins, fnmatch

FILENAME = "<student>"
MARKER = "\x1eSTEPWISE-TRACE\x1e"
MAX_ITEMS = 60      # elements shown per list / dict
MAX_OBJS = 150      # heap objects per step
MAX_TEXT = 120      # characters per value

real_out = sys.__stdout__.buffer
payload = json.loads(sys.stdin.buffer.read().decode("utf-8"))
MAX_STEPS = int(payload.get("max_steps", 1000))
IMPLEMENTATIONS = {"rustpython": "RustPython", "cpython": "CPython"}
ADDR = re.compile(r" at 0x[0-9a-fA-F]+")
HIDDEN = {"__module__", "__dict__", "__weakref__", "__doc__", "__qualname__",
          "__firstlineno__", "__static_attributes__", "__annotations__",
          "__annotate__", "__builtins__", "__name__", "__classcell__"}


class Out:
    """Collects everything the program prints; tracks the size in UTF-8 bytes."""
    encoding = "utf-8"

    def __init__(self):
        self.parts = []
        self.nbytes = 0

    def write(self, s):
        s = str(s).encode("utf-8", "replace").decode("utf-8")
        self.parts.append(s)
        self.nbytes += len(s.encode("utf-8"))
        return len(s)

    def flush(self):
        pass

    def isatty(self):
        return False

    def text(self):
        return "".join(self.parts)


out = Out()
steps = []
ids = {}        # id(obj) -> small stable number
frame_ids = {}  # id(frame) -> call number
counter = [0]


def short(s):
    return s if len(s) <= MAX_TEXT else s[: MAX_TEXT - 1] + "…"


def safe_repr(o):
    try:
        return short(ADDR.sub("", repr(o)))
    except BaseException as e:
        return "<%s (repr failed: %s)>" % (type(o).__name__, type(e).__name__)


def name_val(s):
    return {"p": str(s), "k": "name"}


def encode(o, heap):
    t = type(o)
    if o is None or t in (bool, int, float, complex, str):
        return {"p": safe_repr(o), "k": t.__name__}
    key = id(o)
    sid = ids.get(key)
    if sid is None:
        # No reference to the object is kept, so that __del__ runs when the
        # program drops it, exactly as it would outside the visualizer.
        sid = ids[key] = len(ids) + 1
    if sid not in heap:
        if len(heap) >= MAX_OBJS:
            return {"p": "…", "k": "more"}
        heap[sid] = {"kind": "other", "type": t.__name__, "text": "…"}
        try:
            heap[sid] = encode_obj(o, t, heap)
        except RecursionError:
            pass
        except Exception:
            heap[sid] = {"kind": "other", "type": t.__name__, "text": safe_repr(o)}
    return {"r": sid}


def is_user_class(c):
    return getattr(c, "__module__", None) == "__main__"


def encode_obj(o, t, heap):
    tn = t.__name__
    if isinstance(o, (list, tuple, set, frozenset)):
        kind = "list" if isinstance(o, list) else "tuple" if isinstance(o, tuple) else "set"
        n = len(o)
        items = []
        for i, x in enumerate(o):
            if i >= MAX_ITEMS:
                break
            items.append(encode(x, heap))
        return {"kind": kind, "type": tn, "items": items, "more": max(0, n - MAX_ITEMS)}
    if isinstance(o, dict):
        pairs = []
        for i, (k, v) in enumerate(list(o.items())):
            if i >= MAX_ITEMS:
                break
            pairs.append([encode(k, heap), encode(v, heap)])
        return {"kind": "dict", "type": tn, "pairs": pairs, "more": max(0, len(o) - MAX_ITEMS)}
    if isinstance(o, types.FunctionType):
        try:
            sig = str(inspect.signature(o))
        except Exception:
            sig = "(...)"
        name = "lambda" if o.__name__ == "<lambda>" else o.__name__
        return {"kind": "function", "type": "function", "text": short(name + sig)}
    if isinstance(o, types.ModuleType):
        return {"kind": "other", "type": "module", "text": o.__name__}
    if isinstance(o, type):
        if not is_user_class(o):
            return {"kind": "other", "type": "class", "text": o.__name__}
        bases = [b.__name__ for b in o.__bases__ if b is not object]
        pairs = [[name_val(k), encode(v, heap)] for k, v in list(vars(o).items()) if show_name(k, v)]
        return {"kind": "class", "type": o.__name__, "text": ", ".join(bases), "pairs": pairs}
    if is_user_class(t):
        attrs = []
        if hasattr(o, "__dict__"):
            attrs += list(vars(o).items())
        for c in t.__mro__:
            slots = c.__dict__.get("__slots__", ())
            for s in ([slots] if isinstance(slots, str) else slots):
                if hasattr(o, s):
                    attrs.append((s, getattr(o, s)))
        pairs = [[name_val(k), encode(v, heap)] for k, v in attrs[:MAX_ITEMS]]
        return {"kind": "instance", "type": tn, "pairs": pairs}
    return {"kind": "other", "type": tn, "text": safe_repr(o)}


def kind_of(v):
    if isinstance(v, type):
        return "class"
    if isinstance(v, (types.FunctionType, types.BuiltinFunctionType, types.MethodType)):
        return "function"
    if isinstance(v, types.ModuleType):
        return "module"
    return type(v).__name__


def show_name(k, v):
    if not isinstance(k, str):
        return True
    if k in HIDDEN:
        return False
    if k.startswith("__") and k.endswith("__"):
        return isinstance(v, types.FunctionType)
    return True


def matches(name, patterns):
    return any(fnmatch.fnmatchcase(name, g) for g in patterns)


def show_var(k, v):
    """Whether a variable is drawn, after the program's `# stepwise:` comments."""
    if not show_name(k, v):
        return False
    if SHOW_NAMES and not matches(k, SHOW_NAMES):
        return False
    if matches(k, HIDE_NAMES):
        return False
    kind = kind_of(v)
    return not any(t in (kind, kind + "s", kind + "es") for t in HIDE_TYPES)


def encode_vars(d, heap):
    return [[str(k), encode(v, heap)] for k, v in list(d.items()) if show_var(k, v)]


def frame_name(code):
    if not (code.co_flags & 0x2):  # no CO_NEWLOCALS: a class body
        return "class " + code.co_name
    return "lambda" if code.co_name == "<lambda>" else code.co_name


def snapshot(frame, event, arg=None, msg=None, line=None):
    heap = {}
    stack = []
    f = frame
    while f is not None:
        if f.f_code.co_filename == FILENAME and f.f_code.co_name != "<module>":
            stack.append(f)
        f = f.f_back
    stack.reverse()
    glb = encode_vars(user_globals, heap)
    frames = []
    for f in stack:
        fid = frame_ids.get(id(f))
        if fid is None:
            counter[0] += 1
            fid = frame_ids[id(f)] = counter[0]
        rec = {"id": fid, "name": frame_name(f.f_code), "line": f.f_lineno,
               "locals": encode_vars(f.f_locals, heap)}
        if event == "return" and f is frame:
            rec["ret"] = encode(arg, heap)
        frames.append(rec)
    steps.append({
        "event": event,
        "line": line if line is not None else (frame.f_lineno if frame is not None else 0),
        "func": frames[-1]["name"] if frames else "",
        "out": out.nbytes,
        "globals": glb,
        "frames": frames,
        "heap": heap,
        "msg": msg,
    })


def finish(extra=None):
    sys.settrace(None)
    result = {"steps": steps, "stdout": out.text(),
              "python": "%s %d.%d.%d" % ((IMPLEMENTATIONS.get(sys.implementation.name, sys.implementation.name),)
                                         + tuple(sys.version_info[:3]))}
    if extra:
        result.update(extra)
    data = MARKER + json.dumps(result, ensure_ascii=False)
    real_out.write(data.encode("utf-8", "replace"))
    real_out.flush()
    os._exit(0)


def check_time():
    # Stop with what was recorded so far, instead of being killed with nothing.
    if time.time() - started > MAX_SECONDS:
        finish({"limit": "time"})


def program_ended(event, msg=None, line=None):
    snapshot(None, event, msg=msg, line=line if line is not None else (steps[-1]["line"] if steps else 0))
    # What Python does when a program ends: drop every object, which runs
    # finalizers and closes generators. Anything they print belongs to the end.
    sys.settrace(None)
    try:
        user_globals.clear()
        import gc
        gc.collect()
    except BaseException:
        pass
    steps[-1]["out"] = out.nbytes


def tracer(frame, event, arg):
    code = frame.f_code
    if code.co_filename != FILENAME:
        return None
    is_module = code.co_name == "<module>"
    if event == "call":
        if is_module:
            return tracer
        counter[0] += 1
        frame_ids[id(frame)] = counter[0]
    elif event == "return" and is_module:
        return tracer
    if STEP_LINES and not (event == "line" and frame.f_lineno in STEP_LINES):
        check_time()
        return tracer
    msg = None
    if event == "exception":
        if arg[0] in (StopIteration, GeneratorExit):
            return tracer
        msg = short("%s: %s" % (arg[0].__name__, arg[1]))
    if len(steps) >= MAX_STEPS:
        finish({"limit": "steps"})
    check_time()
    snapshot(frame, event, arg, msg)
    if event == "return":
        frame_ids.pop(id(frame), None)
    return tracer


def fake_input(prompt=""):
    out.write(prompt)
    line = sys.stdin.readline()
    if line == "":
        raise EOFError("input() was called but there is no more text in the Input box")
    line = line.rstrip("\n")
    out.write(line + "\n")
    return line


user_builtins = dict(vars(builtins))
user_builtins["input"] = fake_input
user_globals = {"__name__": "__main__", "__builtins__": user_builtins}

source = payload["code"]
MAX_SECONDS = 8
started = time.time()

# Comments in the program that shape the visualization:
#   a line ending with  # step               -> only those lines become steps
#   # stepwise: show letters, total          -> draw only these variables
#   # stepwise: hide i, tmp_*                -> do not draw these variables
#   # stepwise: hide types function, module  -> do not draw variables holding these
# Names accept * as a wildcard.
STEP_LINES = set()
SHOW_NAMES = []
HIDE_NAMES = []
HIDE_TYPES = set()
for number, text in enumerate(source.split("\n"), 1):
    if re.search(r"#\s*step\s*$", text, re.I):
        STEP_LINES.add(number)
    m = re.match(r"\s*#\s*stepwise\s*:\s*(show|hide\s+types?|hide)\b\s*:?(.*)$", text, re.I)
    if m:
        names = [n.strip() for n in m.group(2).split(",") if n.strip()]
        action = " ".join(m.group(1).lower().split())
        if action == "show":
            SHOW_NAMES.extend(names)
        elif action == "hide":
            HIDE_NAMES.extend(names)
        else:
            HIDE_TYPES.update(n.lower() for n in names)
try:
    program = compile(source, FILENAME, "exec")
except SyntaxError as e:
    finish({"error": {"msg": "SyntaxError: %s" % e.msg, "line": e.lineno or 0}})

sys.stdin = io.StringIO(payload.get("stdin", ""))
sys.stdout = sys.stderr = out
sys.argv = ["main.py"]
sys.setrecursionlimit(6000)

try:
    sys.settrace(tracer)
    try:
        exec(program, user_globals)
    finally:
        sys.settrace(None)
except SystemExit:
    program_ended("end")
except BaseException as e:
    line = steps[-1]["line"] if steps else 0
    tb = e.__traceback__
    while tb is not None:
        if tb.tb_frame.f_code.co_filename == FILENAME:
            line = tb.tb_lineno
            break
        tb = tb.tb_next
    msg = short("%s: %s" % (type(e).__name__, e))
    del e, tb
    program_ended("uncaught", msg=msg, line=line)
else:
    program_ended("end")
finish()
