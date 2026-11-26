from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="PageSpeechVoiceDataItem")



@_attrs_define
class PageSpeechVoiceDataItem:
    """ A voice a provider speaks.

        Attributes:
            name (str):
            display_name (str):
            language (str):
            gender (None | str | Unset):
     """

    name: str
    display_name: str
    language: str
    gender: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        name = self.name

        display_name = self.display_name

        language = self.language

        gender: None | str | Unset
        if isinstance(self.gender, Unset):
            gender = UNSET
        else:
            gender = self.gender


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "name": name,
            "display_name": display_name,
            "language": language,
        })
        if gender is not UNSET:
            field_dict["gender"] = gender

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        name = d.pop("name")

        display_name = d.pop("display_name")

        language = d.pop("language")

        def _parse_gender(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        gender = _parse_gender(d.pop("gender", UNSET))


        page_speech_voice_data_item = cls(
            name=name,
            display_name=display_name,
            language=language,
            gender=gender,
        )


        page_speech_voice_data_item.additional_properties = d
        return page_speech_voice_data_item

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
