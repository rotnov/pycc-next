# pycc's compile script (`COMPILE_SCRIPT`, src/ext_build/precompiled.rs): run
# with `-I -c` by the target interpreter; stdin and stdout are described there.
import sys,marshal,importlib.util,__future__
from opcode import opmap as O
def walk(co,acc):
  for k in co.co_consts:
    if type(k) is frozenset and len(k)>1: acc.append(k)
    elif hasattr(k,'co_consts'): walk(k,acc)
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
  for k in sets:
    found={t for t in shown if frozenset(t)==k}
    if len(found)!=1: raise ValueError
    out.append(found.pop())
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
# The `def` entry of chunk code `co`, `((spec, ...),)`, when the chunk is
# only `def` statements with constant defaults, keyword defaults and
# annotations (no decorator, no closure), each perhaps followed by its
# `__pycc_forward__(...)` line, or a nested statement's scaffold with its
# `__pycc_fallback_nestspec` call; else None. A spec is `[code, defaults,
# kwdefaults, annotations, name, forward]` for a `def` (what MAKE_FUNCTION
# and SET_FUNCTION_ATTRIBUTE take, the STORE_NAME, the forwarded names), or
# `[name, code, order]`, the scaffold's `nestspec` answer bound as `name`.
# Read by symbolic execution of the 3.13 bytecode: a stack of constants
# (`[value]`) and functions; any other instruction makes it None.
def defs(co):
  if sys.version_info[:2]!=(3,13): return None
  k,n=co.co_consts,co.co_names
  it=list(ops(co));st=[];out=[];i=0
  if not it or it[0][0]!=O['RESUME']: return None
  it=it[1:]
  while i<len(it):
    o,a=it[i];i+=1
    if o==O['LOAD_CONST']: st.append([k[a]])
    elif o==O['BUILD_TUPLE'] and a<=len(st) and all(len(v)==1 for v in st[len(st)-a:]):
      v=tuple(x[0] for x in st[len(st)-a:]);del st[len(st)-a:];st.append([v])
    elif o==O['BUILD_CONST_KEY_MAP'] and a<len(st) and all(len(v)==1 for v in st[len(st)-a-1:]) and type(st[-1][0]) is tuple and len(st[-1][0])==a:
      ks=st.pop()[0];v=dict(zip(ks,(x[0] for x in st[len(st)-a:])));del st[len(st)-a:];st.append([v])
    elif o==O['MAKE_FUNCTION'] and st and len(st[-1])==1 and type(st[-1][0]) is type(co) and not st[-1][0].co_freevars:
      st.append([st.pop()[0],None,None,None,None,None])
    elif o==O['SET_FUNCTION_ATTRIBUTE'] and a in (1,2,4) and len(st)>1 and len(st[-1])==6 and len(st[-2])==1:
      f=st.pop();f[{1:1,2:2,4:3}[a]]=st.pop()[0];st.append(f)
    elif o==O['STORE_NAME'] and len(st)==1 and len(st[0])==6:
      f=st.pop();f[4]=n[a];out.append(f)
    elif o==O['LOAD_NAME'] and n[a]=='__pycc_forward__' and not st and out and out[-1][5] is None:
      j=i+1
      while j<len(it) and it[j][0]==O['LOAD_CONST'] and type(k[it[j][1]]) is str: j+=1
      if i>=len(it) or it[i][0]!=O['PUSH_NULL'] or j+1>=len(it) or it[j]!=(O['CALL'],j-i-1) or it[j+1][0]!=O['POP_TOP']: return None
      out[-1][5]=tuple(k[a] for _,a in it[i+1:j]);i=j+2
    elif o==O['LOAD_NAME'] and n[a]=='__pycc_fallback_nestspec' and not st and out and len(out[-1])==6 and out[-1][1:4]==[None]*3 and out[-1][5] is None:
      m=out[-1];w=it[i:i+8];x=n.index(m[4])
      if len(w)<8 or w[0][0]!=O['PUSH_NULL'] or w[1]!=(O['LOAD_NAME'],x) or any(v[0]!=O['LOAD_CONST'] for v in w[2:5]) or w[5]!=(O['CALL'],4) or w[6][0]!=O['STORE_NAME'] or w[7]!=(O['DELETE_NAME'],x): return None
      names,old,new=(k[v[1]] for v in w[2:5])
      cs=[c for c in m[0].co_consts if type(c) is type(co)]
      if len(cs)!=1 or type(names) is not tuple or not set(cs[0].co_freevars)<=set(names): return None
      c=requal(cs[0],old,new) if cs[0].co_qualname==old else cs[0]
      out[-1]=[n[w[6][1]],c,tuple(names.index(v) for v in c.co_freevars)];i+=8
    elif o==O['RETURN_CONST'] and k[a] is None and i==len(it) and not st and out and not any(type(c) is frozenset for c in k):
      return (out,)
    else: return None
  return None
chunks,fix=marshal.loads(sys.stdin.buffer.read())
table=[]
for source,filename,future,rewrites in chunks:
  try:
    f=__future__.annotations.compiler_flag if future else 0
    co=rewrite(compile(source,filename,'exec',f,True,0),rewrites)
    fs=orders(source,co);d=defs(co)
    if d: d=(tuple((*f,orders(source,f[0] if len(f)==6 else f[1])) for f in d[0]),)
    table.append(d or (co,fs))
  except Exception:
    table.append(None)
present=bytes(t is not None for t in table)
table.append(compile(fix,'<pycc precompiled>','exec',0,True,0))
sys.stdout.buffer.write(importlib.util.MAGIC_NUMBER+present+marshal.dumps(tuple(table)))
