from decimal import Decimal


class PaymentReconciler:
    def reconcile(self, expected: Decimal, actual: Decimal) -> bool:
        return self.normalize(expected) == self.normalize(actual)

    def normalize(self, value: Decimal) -> Decimal:
        return value.quantize(Decimal("0.01"))

