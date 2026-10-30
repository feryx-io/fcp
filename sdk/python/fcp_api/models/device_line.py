from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.line_kind import LineKind
from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="DeviceLine")



@_attrs_define
class DeviceLine:
    """ A line on a device (`docs/DEVICE_LINES.md` §2): an account it registers
    and takes calls on, as a desk phone's line keys are.

        Attributes:
            kind (LineKind): What a line is.
            value (str | Unset): The extension (`extension`), the number (`shared`) or the user's
                name or id (`on_behalf`); empty for `own`.
            label (None | str | Unset): The administrator's label; unset, the line's default.
            ring (bool | Unset): Whether calls to the line ring this device.
     """

    kind: LineKind
    value: str | Unset = UNSET
    label: None | str | Unset = UNSET
    ring: bool | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        kind = self.kind.value

        value = self.value

        label: None | str | Unset
        if isinstance(self.label, Unset):
            label = UNSET
        else:
            label = self.label

        ring = self.ring


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "kind": kind,
        })
        if value is not UNSET:
            field_dict["value"] = value
        if label is not UNSET:
            field_dict["label"] = label
        if ring is not UNSET:
            field_dict["ring"] = ring

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        kind = LineKind(d.pop("kind"))




        value = d.pop("value", UNSET)

        def _parse_label(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        label = _parse_label(d.pop("label", UNSET))


        ring = d.pop("ring", UNSET)

        device_line = cls(
            kind=kind,
            value=value,
            label=label,
            ring=ring,
        )


        device_line.additional_properties = d
        return device_line

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
