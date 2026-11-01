from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="SoftphoneLine")



@_attrs_define
class SoftphoneLine:
    """ A line on the app (`docs/DEVICE_LINES.md`): an account it registers
    with its own credentials and takes calls on.

        Attributes:
            index (int): 1 for the first; the first is the user's own.
            kind (str): `own`, `extension`, `shared` or `on_behalf`.
            line (str): The line itself, as `PUT /me/lines/{line}/label` names it.
            aor (str): What the app registers the line as.
            label (str): What it is called: the user's name, the administrator's, or
                `default_label`.
            default_label (str):
            shared (bool):
            appearances (int):
            barge_in (bool): A shared line whose active calls the app may join (barge in).
            ring (bool): Whether calls to the line ring the app.
     """

    index: int
    kind: str
    line: str
    aor: str
    label: str
    default_label: str
    shared: bool
    appearances: int
    barge_in: bool
    ring: bool
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        index = self.index

        kind = self.kind

        line = self.line

        aor = self.aor

        label = self.label

        default_label = self.default_label

        shared = self.shared

        appearances = self.appearances

        barge_in = self.barge_in

        ring = self.ring


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "index": index,
            "kind": kind,
            "line": line,
            "aor": aor,
            "label": label,
            "default_label": default_label,
            "shared": shared,
            "appearances": appearances,
            "barge_in": barge_in,
            "ring": ring,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        index = d.pop("index")

        kind = d.pop("kind")

        line = d.pop("line")

        aor = d.pop("aor")

        label = d.pop("label")

        default_label = d.pop("default_label")

        shared = d.pop("shared")

        appearances = d.pop("appearances")

        barge_in = d.pop("barge_in")

        ring = d.pop("ring")

        softphone_line = cls(
            index=index,
            kind=kind,
            line=line,
            aor=aor,
            label=label,
            default_label=default_label,
            shared=shared,
            appearances=appearances,
            barge_in=barge_in,
            ring=ring,
        )


        softphone_line.additional_properties = d
        return softphone_line

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
