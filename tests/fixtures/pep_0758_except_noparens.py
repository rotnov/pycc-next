def raise_value_error() -> None:
    raise ValueError("bad value")


def raise_type_error() -> None:
    raise TypeError("bad type")


def raise_key_error() -> None:
    raise KeyError("missing key")


def bare_comma_catches_value_error() -> str:
    try:
        raise_value_error()
    except ValueError, TypeError:
        return "caught bare comma"
    return "unreached"


def bare_comma_catches_type_error() -> str:
    try:
        raise_type_error()
    except ValueError, TypeError:
        return "caught bare comma"
    return "unreached"


def parenthesized_catches_value_error() -> str:
    try:
        raise_value_error()
    except (ValueError, TypeError):
        return "caught parenthesized"
    return "unreached"


def parenthesized_catches_type_error() -> str:
    try:
        raise_type_error()
    except (ValueError, TypeError):
        return "caught parenthesized"
    return "unreached"


def as_binding_rebinds_and_reraises() -> str:
    try:
        try:
            raise_type_error()
        except (ValueError, TypeError) as e:
            raise e
    except TypeError:
        return "as binding reraised and was recaught"
    return "unreached"


def three_type_handler_catches_key_error() -> str:
    try:
        raise_key_error()
    except (ValueError, TypeError, KeyError):
        return "caught three-type"
    return "unreached"


def non_matching_raise_propagates_to_caller() -> str:
    try:
        raise_key_error()
    except (ValueError, TypeError):
        return "wrongly caught"
    return "unreached"


def non_matching_raise_propagates_through_an_outer_handler() -> str:
    try:
        return non_matching_raise_propagates_to_caller()
    except KeyError:
        return "propagated and caught by caller"


print(bare_comma_catches_value_error())
print(bare_comma_catches_type_error())
print(parenthesized_catches_value_error())
print(parenthesized_catches_type_error())
print(as_binding_rebinds_and_reraises())
print(three_type_handler_catches_key_error())
print(non_matching_raise_propagates_through_an_outer_handler())
