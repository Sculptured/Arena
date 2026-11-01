"""What the plant and the mirror share: documents of instances and saves, inspect, diff and the save folders."""
from arena_part import Part, errors, saves

from . import views


class DocsPart(Part):
    name = ""  # the package name; saves are at $ARENA_SAVES/<name>/<save>/

    def __init__(self):
        self.instances = {}

    def _instance(self, instance):
        try:
            return self.instances[instance]
        except KeyError:
            raise errors.unknown(f"unknown instance {instance}") from None

    def _live_docs(self, instance) -> dict:
        raise NotImplementedError

    def _docs(self, target) -> dict:
        if "instance" in target:
            return self._live_docs(target["instance"])
        if "save" in target:
            return views.read_docs(saves.root(self.name) / target["save"])
        raise errors.invalid("invalid params: target needs instance or save")

    def inspect(self, target, keys):
        return {"docs": views.pick(self._docs(target), keys)}

    def diff(self, a, b):
        return {"differing": views.differing(self._docs(a), self._docs(b))}

    def drop_save(self, save):
        saves.drop(self.name, save)

    def stop(self, instance):
        self.instances.pop(instance, None)

    def usage(self):
        return {"bytes": 0}  # every save lives under ARENA_SAVES, which arenad measures itself

    def gc(self, keep):
        return saves.gc(self.name, keep)
