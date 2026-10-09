import sys

pl=1
sys.setrecursionlimit(10**5)
if pl:
	input=sys.stdin.readline
else:	
	sys.stdin=open('input.txt', 'r')
	sys.stdout=open('outpt.txt','w')

def li():
	return [int(xxx) for xxx in input().split()]
def fi():
	return int(input())
def si():
	return list(input().rstrip())	
def mi():
	return 	map(int,input().split())	
def ff():
	sys.stdout.flush()
def google(tc,*ans):
	print("Case #"+str(tc)+":",*ans)	
def bits(i,n):
	p=bin(i)[2:]
	return (n-len(p))*"0"+p			

t=1
f=t	
mod=10**9+7	


while t:
	t-=1
	n,m=mi()
	a=[]
	for i in range(n):
		s=si()
		p=""
		for j in range(m):
			if j%2==0:
				p+=s[j]
			else:
				w=ord(s[j])-ord('a')
				p+=chr(ord('a')+25-w)
		a.append([p,i+1])
	a.sort()
	for i in a:
		print(i[1],end=" ")						
