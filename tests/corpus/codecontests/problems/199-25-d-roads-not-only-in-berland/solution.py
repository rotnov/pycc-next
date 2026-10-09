import sys

lines = sys.stdin.read().split("\n")
lines.pop(-1)
n = int(lines.pop(0))
 
#Saque esta estructura de internet porque no habia otra
#implementacion nativa en python y no queria calentarme
class Node:
    """Represents an element of a set."""
    def __init__(self, id):
        self.id = id
        self.parent = self
        self.rank = 0
        self.size = 1
 
    def __repr__(self):
        return 'Node({!r})'.format(self.id)
 
 
def Find(x):
    """Returns the representative object of the set containing x."""
    if x.parent is not x:
        x.parent = Find(x.parent)
    return x.parent
 
 
def Union(x, y):
    """Combines the sets x and y belong to."""
    xroot = Find(x)
    yroot = Find(y)
 
    # x and y are already in the same set
    if xroot is yroot:
        return
 
    # x and y are not in same set, so we merge them
    if xroot.rank < yroot.rank:
        xroot, yroot = yroot, xroot  # swap xroot and yroot
 
    # merge yroot into xroot
    yroot.parent = xroot
    xroot.size += yroot.size
    if xroot.rank == yroot.rank:
        xroot.rank = xroot.rank + 1
 
nodos = [Node(i) for i in range(n+1)]
edges = []
final = []
for line in lines:
    a = nodos[int(line.split(" ")[0])]
    b = nodos[int(line.split(" ")[1])]
    if (Find(a) != Find(b)):
        Union(a, b)
    else:
        edges.append( (a,b) )
 
for i in range(2,n+1):
    if (Find(nodos[1]) != Find(nodos[i])):
        final.append( ( nodos[1],nodos[i] ) )
        Union(nodos[1],nodos[i])
        
print(len(final))
for i in range(len(final)):
    print(edges[i][0].id, end=" ")
    print(edges[i][1].id, end=" ")
    print(final[i][0].id, end=" ")
    print(final[i][1].id, end=" ")
