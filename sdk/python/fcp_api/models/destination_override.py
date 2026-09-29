from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.destination_class import DestinationClass






T = TypeVar("T", bound="DestinationOverride")



@_attrs_define
class DestinationOverride:
    """ A tenant's own word on a range of numbers: `prefix` is E.164 when it
    starts with `+` (`+1900`), or the digits as dialled otherwise (`118`).
    The longest matching override wins over the built-in table.

        Attributes:
            prefix (str):
            class_ (DestinationClass): What kind of destination a dialled number is.
     """

    prefix: str
    class_: DestinationClass
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        prefix = self.prefix

        class_ = self.class_.value


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "prefix": prefix,
            "class": class_,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        prefix = d.pop("prefix")

        class_ = DestinationClass(d.pop("class"))




        destination_override = cls(
            prefix=prefix,
            class_=class_,
        )


        destination_override.additional_properties = d
        return destination_override

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
