def time(sec):
    h = sec // 3600
    ti = sec % 3600
    m = ti // 60
    s = ti % 60
    return [h,m,s]

while True:
    t,h,s = map(int,input().split())
    if t == h == s == -1:
        break
    sec = t*3600 + h * 60 + s
    sec = 7200 - sec
    ans = time(sec)
    t_ans = time(3 * sec)
    if ans[2] < 10:
        ans[2] = "0" + str(ans[2])
    if ans[1] < 10:
        ans[1] = "0" + str(ans[1])
    print("0"+ str(ans[0])+ ":" + str(ans[1]) + ":" +str(ans[2]))
    if t_ans[2] < 10:
        t_ans[2] = "0" + str(t_ans[2])
    if t_ans[1] < 10:
        t_ans[1] = "0" + str(t_ans[1])

    print("0"+ str(t_ans[0])+ ":" + str(t_ans[1]) + ":" +str(t_ans[2]))
