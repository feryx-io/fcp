from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="UpdatePromptRequest")



@_attrs_define
class UpdatePromptRequest:
    """ 
        Attributes:
            description (None | str | Unset):
            category (None | str | Unset):
            language (None | str | Unset):
            tts_text (None | str | Unset):
            tts_voice (None | str | Unset):
     """

    description: None | str | Unset = UNSET
    category: None | str | Unset = UNSET
    language: None | str | Unset = UNSET
    tts_text: None | str | Unset = UNSET
    tts_voice: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        description: None | str | Unset
        if isinstance(self.description, Unset):
            description = UNSET
        else:
            description = self.description

        category: None | str | Unset
        if isinstance(self.category, Unset):
            category = UNSET
        else:
            category = self.category

        language: None | str | Unset
        if isinstance(self.language, Unset):
            language = UNSET
        else:
            language = self.language

        tts_text: None | str | Unset
        if isinstance(self.tts_text, Unset):
            tts_text = UNSET
        else:
            tts_text = self.tts_text

        tts_voice: None | str | Unset
        if isinstance(self.tts_voice, Unset):
            tts_voice = UNSET
        else:
            tts_voice = self.tts_voice


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if description is not UNSET:
            field_dict["description"] = description
        if category is not UNSET:
            field_dict["category"] = category
        if language is not UNSET:
            field_dict["language"] = language
        if tts_text is not UNSET:
            field_dict["tts_text"] = tts_text
        if tts_voice is not UNSET:
            field_dict["tts_voice"] = tts_voice

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        def _parse_description(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        description = _parse_description(d.pop("description", UNSET))


        def _parse_category(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        category = _parse_category(d.pop("category", UNSET))


        def _parse_language(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        language = _parse_language(d.pop("language", UNSET))


        def _parse_tts_text(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        tts_text = _parse_tts_text(d.pop("tts_text", UNSET))


        def _parse_tts_voice(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        tts_voice = _parse_tts_voice(d.pop("tts_voice", UNSET))


        update_prompt_request = cls(
            description=description,
            category=category,
            language=language,
            tts_text=tts_text,
            tts_voice=tts_voice,
        )


        update_prompt_request.additional_properties = d
        return update_prompt_request

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
