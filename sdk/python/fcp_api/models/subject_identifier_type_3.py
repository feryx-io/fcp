from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.subject_identifier_type_3_type import SubjectIdentifierType3Type






T = TypeVar("T", bound="SubjectIdentifierType3")



@_attrs_define
class SubjectIdentifierType3:
    """ 
        Attributes:
            address (str):
            type_ (SubjectIdentifierType3Type):
     """

    address: str
    type_: SubjectIdentifierType3Type
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        address = self.address

        type_ = self.type_.value


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "address": address,
            "type": type_,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        address = d.pop("address")

        type_ = SubjectIdentifierType3Type(d.pop("type"))




        subject_identifier_type_3 = cls(
            address=address,
            type_=type_,
        )


        subject_identifier_type_3.additional_properties = d
        return subject_identifier_type_3

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
