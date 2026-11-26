from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.speech_settings_request_voices import SpeechSettingsRequestVoices





T = TypeVar("T", bound="SpeechSettingsRequest")



@_attrs_define
class SpeechSettingsRequest:
    """ The settings to keep; null or absent is the platform's default.

        Attributes:
            provider (None | str | Unset):
            voice (None | str | Unset):
            language (None | str | Unset):
            voices (SpeechSettingsRequestVoices | Unset):
     """

    provider: None | str | Unset = UNSET
    voice: None | str | Unset = UNSET
    language: None | str | Unset = UNSET
    voices: SpeechSettingsRequestVoices | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.speech_settings_request_voices import SpeechSettingsRequestVoices # noqa: PLC0415
        provider: None | str | Unset
        if isinstance(self.provider, Unset):
            provider = UNSET
        else:
            provider = self.provider

        voice: None | str | Unset
        if isinstance(self.voice, Unset):
            voice = UNSET
        else:
            voice = self.voice

        language: None | str | Unset
        if isinstance(self.language, Unset):
            language = UNSET
        else:
            language = self.language

        voices: dict[str, Any] | Unset = UNSET
        if not isinstance(self.voices, Unset):
            voices = self.voices.to_dict()


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if provider is not UNSET:
            field_dict["provider"] = provider
        if voice is not UNSET:
            field_dict["voice"] = voice
        if language is not UNSET:
            field_dict["language"] = language
        if voices is not UNSET:
            field_dict["voices"] = voices

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.speech_settings_request_voices import SpeechSettingsRequestVoices # noqa: PLC0415
        d = dict(src_dict)
        def _parse_provider(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        provider = _parse_provider(d.pop("provider", UNSET))


        def _parse_voice(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        voice = _parse_voice(d.pop("voice", UNSET))


        def _parse_language(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        language = _parse_language(d.pop("language", UNSET))


        _voices = d.pop("voices", UNSET)
        voices: SpeechSettingsRequestVoices | Unset
        if isinstance(_voices,  Unset):
            voices = UNSET
        else:
            voices = SpeechSettingsRequestVoices.from_dict(_voices)




        speech_settings_request = cls(
            provider=provider,
            voice=voice,
            language=language,
            voices=voices,
        )


        speech_settings_request.additional_properties = d
        return speech_settings_request

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
