# pycc's compile script (`COMPILE_SCRIPT`, src/ext_build/precompiled.rs): run
# with `-I -c` by the target interpreter; stdin and stdout are described there.
import sys,marshal,importlib.util,__future__,opcode
from opcode import opmap as O
# Every `frozenset` constant of `co` (and its nested code), with the path of
# `co_consts` indices that leads to it, appended to `acc` as `(path, set)`.
def walk(co,acc,path=()):
  for i,k in enumerate(co.co_consts):
    if type(k) is frozenset and len(k)>1: acc.append((path+(i,),k))
    elif hasattr(k,'co_consts'): walk(k,acc,path+(i,))
def displays(src):
  import ast
  out=[]
  for n in ast.walk(ast.parse(src)):
    if isinstance(n,ast.Set):
      try: out.append(tuple(ast.literal_eval(e) for e in n.elts))
      except Exception: pass
  return out
def orders(src,co):
  sets=[]
  walk(co,sets)
  if not sets: return None
  shown=displays(src)
  out=[]
  for p,k in sets:
    found={t for t in shown if frozenset(t)==k}
    if len(found)!=1: raise ValueError
    out.append((p,found.pop()))
  return tuple(out)
def requal(c,old,new):
  ks=tuple(requal(k,old,new) if hasattr(k,'co_code') else k for k in c.co_consts)
  q=c.co_qualname
  if q==old: q=new or c.co_name
  elif q.startswith(old+'.<locals>.'): q=(new+'.<locals>.' if new else '')+q[len(old)+10:]
  return c.replace(co_consts=ks,co_qualname=q)
def at(c,path,op):
  if not path: return op(c)
  return c.replace(co_consts=tuple(at(k,path[1:],op) if hasattr(k,'co_code') and k.co_name==path[0] else k for k in c.co_consts))
def rewrite(co,rewrites):
  for kind,path,a,b in rewrites:
    if kind=='q': co=at(co,path,lambda c:requal(c,a,b))
    else: co=at(co,path,lambda c:c.replace(co_qualname=a,co_flags=c.co_flags|b))
  return co
def ops(co):
  b=co.co_code;e=0
  for i in range(0,len(b),2):
    o,a=b[i],b[i+1]|e
    if o==O['EXTENDED_ARG']: e=a<<8;continue
    e=0
    if o!=O['CACHE'] and o!=O['NOP']: yield o,a
# The constant an instruction loads, as `[value]`, or None: `LOAD_CONST`,
# and 3.14+'s `LOAD_SMALL_INT` and `LOAD_COMMON_CONSTANT` (of a plain value).
CC=getattr(opcode,'_common_constants',())
def cv(o,a,k):
  if o==O['LOAD_CONST']: return [k[a]]
  if o==O.get('LOAD_SMALL_INT'): return [a]
  if o==O.get('LOAD_COMMON_CONSTANT') and a<len(CC) and type(CC[a]) in (type(None),bool,str,int): return [CC[a]]
  return None
# The `def` entry of chunk code `co`, `((spec, ...),)`, when the chunk is
# only `def` statements with constant defaults, keyword defaults and
# annotations (no decorator, no closure), each perhaps followed by its
# `__pycc_forward__(...)` line, or a nested statement's scaffold with its
# `__pycc_fallback_nestspec` call; else None. A spec is `[code, defaults,
# kwdefaults, annotations, name, forward]` for a `def` (what MAKE_FUNCTION
# and SET_FUNCTION_ATTRIBUTE take, the STORE_NAME, the forwarded names), or
# `[name, code, order]`, the scaffold's `nestspec` answer bound as `name`.
# The annotations are 3.13's tuple of names and values, or 3.14+'s
# `__annotate__` code (PEP 649: closure-free at module level, its
# function made as MAKE_FUNCTION makes it). Read by symbolic execution of
# 3.13's or 3.15's bytecode: a stack of constants (`[value]`) and
# functions; any other instruction makes it None.
def defs(co):
  if sys.version_info[:2] not in ((3,13),(3,15)): return None
  k,n=co.co_consts,co.co_names
  it=list(ops(co));st=[];out=[];i=0
  if not it or it[0][0]!=O['RESUME']: return None
  it=it[1:]
  while i<len(it):
    o,a=it[i];i+=1
    c=cv(o,a,k)
    if c is not None: st.append(c)
    elif o==O['BUILD_TUPLE'] and a<=len(st) and all(len(v)==1 for v in st[len(st)-a:]):
      v=tuple(x[0] for x in st[len(st)-a:]);del st[len(st)-a:];st.append([v])
    elif o==O.get('BUILD_CONST_KEY_MAP') and a<len(st) and all(len(v)==1 for v in st[len(st)-a-1:]) and type(st[-1][0]) is tuple and len(st[-1][0])==a:
      ks=st.pop()[0];v=dict(zip(ks,(x[0] for x in st[len(st)-a:])));del st[len(st)-a:];st.append([v])
    elif o==O['BUILD_MAP'] and 2*a<=len(st) and all(len(v)==1 for v in st[len(st)-2*a:]):
      w=[x[0] for x in st[len(st)-2*a:]];del st[len(st)-2*a:];st.append([dict(zip(w[0::2],w[1::2]))])
    elif o==O['MAKE_FUNCTION'] and st and len(st[-1])==1 and type(st[-1][0]) is type(co) and not st[-1][0].co_freevars:
      st.append([st.pop()[0],None,None,None,None,None])
    elif o==O['SET_FUNCTION_ATTRIBUTE'] and a in (1,2,4) and len(st)>1 and len(st[-1])==6 and len(st[-2])==1:
      f=st.pop();f[{1:1,2:2,4:3}[a]]=st.pop()[0];st.append(f)
    elif o==O['SET_FUNCTION_ATTRIBUTE'] and a==16 and len(st)>1 and len(st[-1])==6 and len(st[-2])==6 and st[-2][1:]==[None]*5:
      f=st.pop();g=st.pop()[0];s2=[]
      walk(g,s2)
      if s2: return None
      f[3]=g;st.append(f)
    elif o==O['STORE_NAME'] and len(st)==1 and len(st[0])==6:
      f=st.pop();f[4]=n[a];out.append(f)
    elif o==O['LOAD_NAME'] and n[a]=='__pycc_forward__' and not st and out and out[-1][5] is None:
      j=i+1
      while j<len(it) and it[j][0]==O['LOAD_CONST'] and type(k[it[j][1]]) is str: j+=1
      if i>=len(it) or it[i][0]!=O['PUSH_NULL'] or j+1>=len(it) or it[j]!=(O['CALL'],j-i-1) or it[j+1][0]!=O['POP_TOP']: return None
      out[-1][5]=tuple(k[a] for _,a in it[i+1:j]);i=j+2
    elif o==O['LOAD_NAME'] and n[a]=='__pycc_fallback_nestspec' and not st and out and len(out[-1])==6 and out[-1][1:4]==[None]*3 and out[-1][5] is None:
      m=out[-1];w=it[i:i+8];x=n.index(m[4])
      if len(w)<8 or w[0][0]!=O['PUSH_NULL'] or w[1]!=(O['LOAD_NAME'],x) or any(cv(*v,k) is None for v in w[2:5]) or w[5]!=(O['CALL'],4) or w[6][0]!=O['STORE_NAME'] or w[7]!=(O['DELETE_NAME'],x): return None
      names,old,new=(cv(*v,k)[0] for v in w[2:5])
      cs=[c for c in m[0].co_consts if type(c) is type(co)]
      if len(cs)!=1 or type(names) is not tuple or not set(cs[0].co_freevars)<=set(names): return None
      c=requal(cs[0],old,new) if cs[0].co_qualname==old else cs[0]
      out[-1]=[n[w[6][1]],c,tuple(names.index(v) for v in c.co_freevars)];i+=8
    elif ((o==O.get('RETURN_CONST') and k[a] is None and not st) or (o==O['RETURN_VALUE'] and st==[[None]])) and i==len(it) and out and not any(type(c) is frozenset for c in k):
      return (out,)
    else: return None
  return None
# A signature wrapper chunk's entry (`src/ext_build/signatures.rs`),
# `(code, defaults, kwdefaults, cells)`: the function `_pycc_make` returns,
# which the shim then makes itself, each closure cell holding the factory's
# parameter `SIG[cells[i]]`. Only when the source is `_pycc_make` and nothing else, its
# body only the wrapper `def` (no decorator, annotation or twin builder)
# with constant defaults and the two lines naming it, and the wrapper's
# code (already given its qualified name and flag) closes over the
# factory's parameters alone and holds no set constant; else None.
SIG=('_pycc_entry','_pycc_missing','_pycc_tuple','_pycc_isinstance','_pycc_type','_pycc_TypeError','_pycc_int','_pycc_float','_pycc_str','_pycc_bool','_pycc_BaseException')
def sig(src,co):
  import ast
  m=ast.parse(src).body
  if len(m)!=1 or type(m[0]) is not ast.FunctionDef or m[0].name!='_pycc_make' or m[0].decorator_list: return None
  if [p.arg for p in m[0].args.args]!=list(SIG) or m[0].args.kwonlyargs or m[0].args.vararg or m[0].args.kwarg: return None
  b=m[0].body
  if [type(x) for x in b]!=[ast.FunctionDef,ast.Assign,ast.If,ast.Return]: return None
  f=b[0];a=f.args;ps=a.posonlyargs+a.args+a.kwonlyargs+[v for v in (a.vararg,a.kwarg) if v]
  if f.decorator_list or f.returns or getattr(f,'type_params',None) or any(p.annotation for p in ps): return None
  ds=a.defaults;ks=a.kw_defaults
  if not all(type(d) is ast.Constant for d in ds+[d for d in ks if d is not None]): return None
  mk=[k for k in co.co_consts if hasattr(k,'co_code')]
  if len(mk)!=1 or mk[0].co_name!='_pycc_make': return None
  fs=[k for k in mk[0].co_consts if hasattr(k,'co_code')]
  if len(fs)!=1 or fs[0].co_name!=f.name or not set(fs[0].co_freevars)<=set(SIG) or not fs[0].co_flags&0x40000000: return None
  q=b[1].value
  if type(q) is not ast.Constant or q.value!=fs[0].co_qualname: return None
  s2=[];walk(fs[0],s2)
  if s2: return None
  return (fs[0],tuple(d.value for d in ds) or None,{p.arg:d.value for p,d in zip(a.kwonlyargs,ks) if d is not None} or None,bytes(SIG.index(v) for v in fs[0].co_freevars))
# v0.12: the texts `dataclasses` generates for the chunk's dataclasses,
# `{text: code}`, each compiled as `exec` of the text compiles it (file
# `<string>`, no flags), or `(code, orders)` when it holds `frozenset`
# constants (a frozen dataclass's field names), or a list of those for a
# text several classes generate; None when there is none. Each class statement
# decorated last by a `dataclass` (or `x.dataclass`) with constant keywords
# gets a stand-in -- its fields (a default or a `field()` of constant
# flags as placeholders), the names its body binds, bases defined before it
# in the module -- and the target interpreter's own `dataclasses` makes its
# methods, its `exec` calls recorded. Only a guess: the import uses a code
# only for a text equal to the one `dataclasses` generates there
# (`precompiled_hook.c`), so a wrong guess costs the compile it would have.
DC={}
def dcs(src):
  import ast,builtins,dataclasses,typing
  class S:
    __slots__=()
  def F(): pass
  def stub(self,*a,**k): pass
  def name(e):
    return e.id if type(e) is ast.Name else e.attr if type(e) is ast.Attribute else None
  def head(a):
    if type(a) is ast.Constant and type(a.value) is str:
      try: a=ast.parse(a.value,mode='eval').body
      except SyntaxError: return None
    return name(a.value if type(a) is ast.Subscript else a)
  def lit(e):
    try: return [ast.literal_eval(e)]
    except Exception: return None
  def standin(c):
    d=c.decorator_list[-1] if c.decorator_list else None
    f=d.func if type(d) is ast.Call else d
    if c.keywords or d is None or name(f)!='dataclass': return None
    kw={}
    for k in (d.keywords if type(d) is ast.Call else ()):
      v=lit(k.value)
      if k.arg is None or v is None: return None
      kw[k.arg]=v[0]
    if type(d) is ast.Call and d.args: return None
    bases=[]
    for b in c.bases:
      k=DC.get(b.id) if type(b) is ast.Name else None
      if k is not None and k not in bases: bases.append(k)
    ns={'__module__':'__pycc_dc__','__qualname__':c.name};ann={}
    body=list(c.body)
    while body:
      s=body.pop(0);t=type(s)
      if t is ast.If:
        g=s.test;neg=type(g) is ast.UnaryOp and type(g.op) is ast.Not
        if neg: g=g.operand
        if type(g) not in (ast.Name,ast.Attribute) or name(g)!='TYPE_CHECKING': return None
        body[:0]=s.body if neg else s.orelse;continue
      if t in (ast.Expr,ast.Pass): continue
      if t in (ast.FunctionDef,ast.AsyncFunctionDef,ast.ClassDef):
        ns[s.name]=stub;continue
      if t is ast.Assign:
        v=lit(s.value)
        for g in s.targets:
          if type(g) is not ast.Name: return None
          ns[g.id]=v[0] if v else S()
        continue
      if t is not ast.AnnAssign or type(s.target) is not ast.Name: return None
      n=s.target.id;h=head(s.annotation)
      ann[n]=typing.ClassVar[int] if h=='ClassVar' else dataclasses.InitVar[int] if h=='InitVar' else dataclasses.KW_ONLY if h=='KW_ONLY' else int
      v=s.value
      if v is None: continue
      if type(v) is ast.Call and name(v.func)=='field':
        if v.args: return None
        fk={}
        for k in v.keywords:
          if k.arg is None: return None
          if k.arg=='default': fk[k.arg]=S()
          elif k.arg=='default_factory': fk[k.arg]=F
          elif k.arg not in ('metadata','doc'):
            l=lit(k.value)
            if l is None: return None
            fk[k.arg]=l[0]
        ns[n]=dataclasses.field(**fk)
      else: ns[n]=S()
    ns['__annotations__']=ann
    return bases,ns,kw
  seen=[]
  def record(s,g=None,l=None,/):
    seen.append(s);return builtins.exec(s,g,l)
  for c in ast.parse(src).body:
    st=standin(c) if type(c) is ast.ClassDef else None
    if st is None: continue
    dataclasses.exec=record
    try: DC[c.name]=dataclasses.dataclass(**st[2])(type(c.name,tuple(st[0]),st[1]))
    except Exception: pass
    finally: del dataclasses.exec
  out={}
  for s in seen:
    if type(s) is not str: continue
    co=quiet(s,'exec')
    if co is None: continue
    try: fs=orders(s,co)
    except ValueError: continue
    out.setdefault(s,[]).append(co if fs is None else (co,fs))
  return once(out)
# The code `compile(text, '<string>', mode)` makes, or None when compiling it
# warns (a `SyntaxWarning` for an invalid escape, say): that text is left to
# compile at import, where CPython's warning (or the error a filter makes of
# it) happens as it does there. So is a text with a run of more than 640
# digits (an `int` literal `sys.set_int_max_str_digits` could refuse: 640 is
# the lowest limit it takes), whose compiling depends on the import's limit.
def quiet(text,mode):
  import warnings,re
  if re.search(r'[0-9_]{641}',text): return None
  with warnings.catch_warnings(record=True) as w:
    warnings.simplefilter('always')
    co=compile(text,'<string>',mode,0,True,0)
  return None if w else co
# `out` (`{text: [entry, ...]}`, an entry per time the text is made: each use
# takes one, `precompiled_hook.c`) with each one-entry list replaced by its
# entry; None when empty.
def once(out):
  return {k:v[0] if len(v)==1 else v for k,v in out.items()} or None
# v0.12: the forward references `typing` will compile for the chunk's class
# bodies (a `TypedDict` or `NamedTuple` type-checks each annotation, a string
# becoming a forward reference whose code `compile(text, '<string>', 'eval')`
# makes), `{text: code}` as `dcs`'s; None when there is none. Each candidate
# is a `str` stored into `__annotations__` (an annotation under `from
# __future__ import annotations`, or a string one) or a `str` constant of an
# `__annotate__` function (3.14+), as `typing` rewrites it (a `*` unpack).
def fwds(co):
  try:
    m=__import__('annotationlib' if sys.version_info>=(3,14) else 'typing')
    star=getattr(m,'_rewrite_star_unpack',lambda a:a)
  except Exception: return None
  seen=[]
  def scan(c):
    k,n=c.co_consts,c.co_names;it=list(ops(c))
    for j,(o,a) in enumerate(it):
      v=cv(o,a,k)
      if v and type(v[0]) is str and (c.co_name=='__annotate__' or (j+1<len(it) and it[j+1][0]==O['LOAD_NAME'] and n[it[j+1][1]]=='__annotations__')):
        seen.append(v[0])
    for x in k:
      if type(x) is type(c): scan(x)
  scan(co)
  out={}
  for s in seen:
    try:
      t=star(s);c=quiet(t,'eval')
      if c is None: continue
      fs=orders(t,c)
    except (SyntaxError,ValueError): continue
    out.setdefault(t,[]).append(c if fs is None else (c,fs))
  return once(out)
chunks,fix,exact=marshal.loads(sys.stdin.buffer.read())
table=[]
for source,filename,future,rewrites in chunks:
  try:
    f=__future__.annotations.compiler_flag if future else 0
    co=rewrite(compile(source,filename,'exec',f,True,0),rewrites)
    w=sig(source,co) if filename=='<pycc signature>' else None
    if w:
      table.append(w);continue
    fs=orders(source,co);d=defs(co)
    if d: d=(tuple((*f,orders(source,f[0] if len(f)==6 else f[1])) for f in d[0]),)
    x=None if d or exact or 'dataclass' not in source else dcs(source)
    y=None if d or exact or ('TypedDict' not in source and 'NamedTuple' not in source) else fwds(co)
    table.append((co,fs,(x,y)) if x or y else d or (co,fs))
  except Exception:
    table.append(None)
present=bytes(t is not None for t in table)
table.append(compile(fix,'<pycc precompiled>','exec',0,True,0))
sys.stdout.buffer.write(importlib.util.MAGIC_NUMBER+present+marshal.dumps(tuple(table)))
