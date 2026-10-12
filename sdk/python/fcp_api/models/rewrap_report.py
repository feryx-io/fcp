from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast






T = TypeVar("T", bound="RewrapReport")



@_attrs_define
class RewrapReport:
    """ Outcome of moving the key ring to a new KEK.

        Attributes:
            rewrapped (int): Keys the previous KEK wrapped, now wrapped by the current one.
            already_current (int): Keys the current KEK already wrapped.
            unreadable (list[str]): Keys neither KEK here wraps (ids): values sealed with them cannot be
                read until a node with their KEK rewraps them.
     """

    rewrapped: int
    already_current: int
    unreadable: list[str]
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        rewrapped = self.rewrapped

        already_current = self.already_current

        unreadable = self.unreadable




        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "rewrapped": rewrapped,
            "already_current": already_current,
            "unreadable": unreadable,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        rewrapped = d.pop("rewrapped")

        already_current = d.pop("already_current")

        unreadable = cast(list[str], d.pop("unreadable"))


        rewrap_report = cls(
            rewrapped=rewrapped,
            already_current=already_current,
            unreadable=unreadable,
        )


        rewrap_report.additional_properties = d
        return rewrap_report

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
