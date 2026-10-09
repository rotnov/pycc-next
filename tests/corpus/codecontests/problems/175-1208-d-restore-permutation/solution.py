from sys import stdin,stdout

class Tree(object):
    def __init__(self,n):
        self.tree=[0]*(4*n+10)
        self.b=[0]*(n+10)
        self.a=list(map(int,stdin.readline().split()))
        self.n=n
    def update(self,L,C,l,r,rt):
        if l==r:
            self.tree[rt]+=C
            return
        mid=(l+r)//2
        if L<=mid:
            self.update(L,C,l,mid,rt<<1)
        else:
            self.update(L,C,mid+1,r,rt<<1|1)
        self.tree[rt]=self.tree[rt<<1]+self.tree[rt<<1|1]


    def query(self,s,l,r,rt):
        if l==r:
            return l
        mid=(l+r)//2
        if self.tree[rt<<1]>s:
            return self.query(s,l,mid,rt<<1)
        else:
            return self.query(s-self.tree[rt<<1],mid+1,r,rt<<1|1)
    def slove(self):

        for i in range(n):
            self.update(i+1,i+1,1,n,1)
        for i in range(n,0,-1):
            self.b[i]=self.query(self.a[i-1],1,n,1)
            self.update(self.b[i],-self.b[i],1,n,1)
        for i in range(n):
            stdout.write('%d '%(self.b[i+1]))




if __name__ == '__main__':
    n=int(stdin.readline())
    seg=Tree(n)
    seg.slove()
