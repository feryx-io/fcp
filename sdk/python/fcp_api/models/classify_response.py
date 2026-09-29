from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.destination_class import DestinationClass
from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="ClassifyResponse")



@_attrs_define
class ClassifyResponse:
    """ 
        Attributes:
            dialed (str):
            class_ (DestinationClass): What kind of destination a dialled number is.
            e164 (None | str | Unset):
     """

    dialed: str
    class_: DestinationClass
    e164: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        dialed = self.dialed

        class_ = self.class_.value

        e164: None | str | Unset
        if isinstance(self.e164, Unset):
            e164 = UNSET
        else:
            e164 = self.e164


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "dialed": dialed,
            "class": class_,
        })
        if e164 is not UNSET:
            field_dict["e164"] = e164

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        dialed = d.pop("dialed")

        class_ = DestinationClass(d.pop("class"))




        def _parse_e164(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        e164 = _parse_e164(d.pop("e164", UNSET))


        classify_response = cls(
            dialed=dialed,
            class_=class_,
            e164=e164,
        )


        classify_response.additional_properties = d
        return classify_response

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
