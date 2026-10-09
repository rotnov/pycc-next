def main():
    for _ in range(int(input())):
        s = "".join(sorted(input()))
        t = input()
        A = 0
        B = 0
        C = 0
        for c in s:
            if c == 'a': A+=1
            if c == 'b': B+=1
            if c == 'c': C+=1
        if t != "abc" or A*B*C ==0:
            print(s)
        else:
            B, C = 0,0
            D =""
            for c in s:
                if c == 'a':
                    print("a", end="")
                elif c == 'b':
                    B+=1
                elif c == 'c':
                    C+=1
                else:
                    D+=c
            print(C*'c'+B*'b'+D)
main()
