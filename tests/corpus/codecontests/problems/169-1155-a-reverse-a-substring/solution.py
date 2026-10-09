'''

                            Online Python Compiler.
                Code, Compile, Run and Debug python program online.
Write your code in this editor and press "Run" button to execute it.

'''
def main():
    n = input()
    s = input()
    for i in range(len(s)-1):
        if s[i]>s[i+1]:
            print('YES')
            print(i+1, i+2)
            return
    print('NO')
    
main()
