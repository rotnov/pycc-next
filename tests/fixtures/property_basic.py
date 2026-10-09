class Temperature:
    def __init__(self) -> None:
        self._celsius = 0

    @property
    def celsius(self) -> int:
        return self._celsius


t = Temperature()
print(t.celsius)
