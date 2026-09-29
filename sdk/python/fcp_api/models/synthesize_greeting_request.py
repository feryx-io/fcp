from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="SynthesizeGreetingRequest")



@_attrs_define
class SynthesizeGreetingRequest:
    """ 
        Attributes:
            greeting_type (str):
            text (str):
            language (None | str | Unset):
            voice (None | str | Unset):
            expires_in_hours (int | None | Unset):
     """

    greeting_type: str
    text: str
    language: None | str | Unset = UNSET
    voice: None | str | Unset = UNSET
    expires_in_hours: int | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        greeting_type = self.greeting_type

        text = self.text

        language: None | str | Unset
        if isinstance(self.language, Unset):
            language = UNSET
        else:
            language = self.language

        voice: None | str | Unset
        if isinstance(self.voice, Unset):
            voice = UNSET
        else:
            voice = self.voice

        expires_in_hours: int | None | Unset
        if isinstance(self.expires_in_hours, Unset):
            expires_in_hours = UNSET
        else:
            expires_in_hours = self.expires_in_hours


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "greeting_type": greeting_type,
            "text": text,
        })
        if language is not UNSET:
            field_dict["language"] = language
        if voice is not UNSET:
            field_dict["voice"] = voice
        if expires_in_hours is not UNSET:
            field_dict["expires_in_hours"] = expires_in_hours

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        greeting_type = d.pop("greeting_type")

        text = d.pop("text")

        def _parse_language(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        language = _parse_language(d.pop("language", UNSET))


        def _parse_voice(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        voice = _parse_voice(d.pop("voice", UNSET))


        def _parse_expires_in_hours(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        expires_in_hours = _parse_expires_in_hours(d.pop("expires_in_hours", UNSET))


        synthesize_greeting_request = cls(
            greeting_type=greeting_type,
            text=text,
            language=language,
            voice=voice,
            expires_in_hours=expires_in_hours,
        )


        synthesize_greeting_request.additional_properties = d
        return synthesize_greeting_request

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
