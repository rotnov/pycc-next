"""`A` bound by two imports: the class statement between them reads the
first (`a.A`), a scan reading bindings by name would see the second."""
from .a import A


class B(A, OSError):
    pass


from .other import OtherError as A  # noqa: E402, F811
