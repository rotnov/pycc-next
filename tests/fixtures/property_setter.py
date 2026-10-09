class Temperature:
    def __init__(self) -> None:
        self._celsius = 0

    @property
    def celsius(self) -> int:
        return self._celsius

    @celsius.setter
    def celsius(self, value: int) -> None:
        self._celsius = value


t = Temperature()
print(t.celsius)
t.celsius = 25
print(t.celsius)
