"""gRPC-style servicer for the Orders contract."""


class OrdersServicer:
    def GetOrder(self, request, context):
        return {"id": request["id"]}
