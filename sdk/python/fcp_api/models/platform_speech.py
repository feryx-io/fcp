from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast

if TYPE_CHECKING:
  from ..models.offered_provider import OfferedProvider





T = TypeVar("T", bound="PlatformSpeech")



@_attrs_define
class PlatformSpeech:
    """ Which of the TTS server's providers tenants may choose.

        Attributes:
            providers (list[OfferedProvider]):
            held_back (list[str]): Held back from tenants (a provider the server no longer has may
                still be named here).
     """

    providers: list[OfferedProvider]
    held_back: list[str]
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.offered_provider import OfferedProvider # noqa: PLC0415
        providers = []
        for providers_item_data in self.providers:
            providers_item = providers_item_data.to_dict()
            providers.append(providers_item)



        held_back = self.held_back




        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "providers": providers,
            "held_back": held_back,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.offered_provider import OfferedProvider # noqa: PLC0415
        d = dict(src_dict)
        providers = []
        _providers = d.pop("providers")
        for providers_item_data in (_providers):
            providers_item = OfferedProvider.from_dict(providers_item_data)



            providers.append(providers_item)


        held_back = cast(list[str], d.pop("held_back"))


        platform_speech = cls(
            providers=providers,
            held_back=held_back,
        )


        platform_speech.additional_properties = d
        return platform_speech

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
