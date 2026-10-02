from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.provider_settings import ProviderSettings





T = TypeVar("T", bound="IdentityProviderRequest")



@_attrs_define
class IdentityProviderRequest:
    """ A provider written: its settings, and the client secret — absent to
    keep the one it has, empty to remove it.

        Attributes:
            name (str):
            settings (ProviderSettings): A provider's configuration, kept as one JSON document.
            enabled (bool | Unset):
            client_secret (None | str | Unset):
     """

    name: str
    settings: ProviderSettings
    enabled: bool | Unset = UNSET
    client_secret: None | str | Unset = UNSET





    def to_dict(self) -> dict[str, Any]:
        from ..models.provider_settings import ProviderSettings # noqa: PLC0415
        name = self.name

        settings = self.settings.to_dict()

        enabled = self.enabled

        client_secret: None | str | Unset
        if isinstance(self.client_secret, Unset):
            client_secret = UNSET
        else:
            client_secret = self.client_secret


        field_dict: dict[str, Any] = {}

        field_dict.update({
            "name": name,
            "settings": settings,
        })
        if enabled is not UNSET:
            field_dict["enabled"] = enabled
        if client_secret is not UNSET:
            field_dict["client_secret"] = client_secret

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.provider_settings import ProviderSettings # noqa: PLC0415
        d = dict(src_dict)
        name = d.pop("name")

        settings = ProviderSettings.from_dict(d.pop("settings"))




        enabled = d.pop("enabled", UNSET)

        def _parse_client_secret(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        client_secret = _parse_client_secret(d.pop("client_secret", UNSET))


        identity_provider_request = cls(
            name=name,
            settings=settings,
            enabled=enabled,
            client_secret=client_secret,
        )

        return identity_provider_request

