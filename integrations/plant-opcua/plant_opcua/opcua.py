"""The plant's OPC UA server (asyncua) and the client the bridge tools use, on one background event loop.
Each instance gets its own server on a free local port; its nodes mirror the tags. Every write a client
sends goes through the tag rules first (`tags.apply`): a refused write answers a Bad status and changes
nothing, and the reason is kept in Plant2/Interlocks/LastRefusal, as a PLC keeps its last interlock."""
import asyncio
import logging
import socket
import threading

from asyncua import Client, Server, ua

NS_URI = "urn:ridgeline:plant2"
VTYPES = {"String": ua.VariantType.String, "Boolean": ua.VariantType.Boolean, "Double": ua.VariantType.Double,
          "Int64": ua.VariantType.Int64, "StringArray": ua.VariantType.String}

logging.getLogger("asyncua").setLevel(logging.ERROR)

_loop = asyncio.new_event_loop()
threading.Thread(target=_loop.run_forever, daemon=True, name="opcua").start()


def run(coro, timeout=30):
    return asyncio.run_coroutine_threadsafe(coro, _loop).result(timeout)


def _free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def nodeid(path: str) -> str:
    return f"ns=2;s={path}"


class PlantServer:
    """One instance's OPC UA server. `on_write(path, value)` returns None (applied) or a Bad status name;
    `values()` gives every tag's value now."""

    def __init__(self, on_write, values):
        self.on_write, self.values = on_write, values
        self.port = _free_port()
        self.url = f"opc.tcp://127.0.0.1:{self.port}/ridgeline/plant2"
        self.server = None
        self.nodes = {}

    async def start(self, tags: dict, values: dict):
        s = Server()
        await s.init()
        s.set_endpoint(self.url)
        s.set_server_name("Ridgeline Plant 2 (simulated)")
        idx = await s.register_namespace(NS_URI)
        folders = {}
        for path, tag in tags.items():
            parent = await self._folder(s, idx, folders, path.rsplit("/", 1)[0])
            v = values[path]
            var = await parent.add_variable(ua.NodeId(path, idx), ua.QualifiedName(path.rsplit("/", 1)[1], idx),
                                            ua.Variant(v, VTYPES[tag.vtype]))
            await var.write_attribute(ua.AttributeIds.Description,
                                      ua.DataValue(ua.Variant(ua.LocalizedText(tag.description))))
            if tag.write is not None:
                await var.set_writable()
            self.nodes[path] = (var, tag)
        self.server = s
        svc = s.iserver.attribute_service
        self._server_write = svc.write
        svc.write = self._write
        await s.start()

    async def _folder(self, s, idx, folders, path):
        if path in folders:
            return folders[path]
        parent = s.nodes.objects if "/" not in path else await self._folder(s, idx, folders, path.rsplit("/", 1)[0])
        f = await parent.add_folder(ua.NodeId(path, idx), ua.QualifiedName(path.rsplit("/", 1)[-1], idx))
        folders[path] = f
        return f

    def _ours(self, wv) -> bool:
        return isinstance(wv.NodeId.Identifier, str) and wv.NodeId.Identifier in self.nodes

    async def _write(self, params, user=None):
        """Writes to the plant's tags go through the tag rules; the server's own nodes as usual."""
        if not any(self._ours(wv) for wv in params.NodesToWrite):
            return await (self._server_write(params) if user is None else self._server_write(params, user=user))
        out = []
        for wv in params.NodesToWrite:
            path = wv.NodeId.Identifier if self._ours(wv) else ""
            if wv.AttributeId != ua.AttributeIds.Value or not path:
                out.append(ua.StatusCode(ua.StatusCodes.BadNotWritable))
                continue
            bad = self.on_write(path, wv.Value.Value.Value if wv.Value.Value is not None else None)
            out.append(ua.StatusCode(getattr(ua.StatusCodes, bad)) if bad else ua.StatusCode())
        await self.refresh(self.values())
        return out

    async def refresh(self, values: dict):
        aspace = self.server.iserver.aspace
        for path, (var, tag) in self.nodes.items():
            dv = ua.DataValue(ua.Variant(values[path], VTYPES[tag.vtype]))
            await aspace.write_attribute_value(var.nodeid, ua.AttributeIds.Value, dv)

    async def stop(self):
        if self.server is not None:
            await self.server.stop()
            self.server = None


# ---- the client side: what the bridge tools do, over OPC UA


async def browse(url: str, path: str) -> list:
    async with Client(url) as c:
        node = c.get_node(nodeid(path)) if path else await _root(c)
        out = []
        for ch in await node.get_children():
            cls = await ch.read_node_class()
            p = ch.nodeid.Identifier
            item = {"node": p, "kind": "folder" if cls == ua.NodeClass.Object else "tag"}
            if cls == ua.NodeClass.Variable:
                item["value"] = await ch.read_value()
                item["writable"] = bool(ua.ua_binary.test_bit(
                    (await ch.read_attribute(ua.AttributeIds.AccessLevel)).Value.Value, ua.AccessLevel.CurrentWrite))
                item["description"] = (await ch.read_description()).Text
            out.append(item)
        return out


async def _root(c):
    idx = await c.get_namespace_index(NS_URI)
    objects = c.nodes.objects
    return await objects.get_child([f"{idx}:Plant2"])


async def read(url: str, paths: list) -> dict:
    async with Client(url) as c:
        out = {}
        for p in paths:
            try:
                out[p] = await c.get_node(nodeid(p)).read_value()
            except ua.UaStatusCodeError as e:
                out[p] = {"error": type(e).__name__}
        return out


async def write(url: str, path: str, value):
    """None when the write went through; else the Bad status's name."""
    async with Client(url) as c:
        node = c.get_node(nodeid(path))
        try:
            vtype = await node.read_data_type_as_variant_type()
            await node.write_value(ua.DataValue(ua.Variant(value, vtype)))
        except ua.UaStatusCodeError as e:
            return type(e).__name__
        except (TypeError, ValueError) as e:
            return f"BadTypeMismatch ({e})"
        return None
