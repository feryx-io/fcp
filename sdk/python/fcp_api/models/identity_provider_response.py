from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime

if TYPE_CHECKING:
  from ..models.provider_settings import ProviderSettings





T = TypeVar("T", bound="IdentityProviderResponse")



@_attrs_define
class IdentityProviderResponse:
    """ An identity provider, its client secret withheld.

        Attributes:
            id (str):
            tenant_id (str):
            name (str):
            kind (str):
            enabled (bool):
            settings (ProviderSettings): A provider's configuration, kept as one JSON document.
            has_client_secret (bool): Whether a client secret is set (it is never shown).
            created_at (datetime.datetime):
            updated_at (datetime.datetime):
            redirect_uri (None | str | Unset): The address to register with the provider as the redirect URI;
                absent where `[api.sso] public_url` is not set.
     """

    id: str
    tenant_id: str
    name: str
    kind: str
    enabled: bool
    settings: ProviderSettings
    has_client_secret: bool
    created_at: datetime.datetime
    updated_at: datetime.datetime
    redirect_uri: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.provider_settings import ProviderSettings # noqa: PLC0415
        id = self.id

        tenant_id = self.tenant_id

        name = self.name

        kind = self.kind

        enabled = self.enabled

        settings = self.settings.to_dict()

        has_client_secret = self.has_client_secret

        created_at = self.created_at.isoformat()

        updated_at = self.updated_at.isoformat()

        redirect_uri: None | str | Unset
        if isinstance(self.redirect_uri, Unset):
            redirect_uri = UNSET
        else:
            redirect_uri = self.redirect_uri


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "tenant_id": tenant_id,
            "name": name,
            "kind": kind,
            "enabled": enabled,
            "settings": settings,
            "has_client_secret": has_client_secret,
            "created_at": created_at,
            "updated_at": updated_at,
        })
        if redirect_uri is not UNSET:
            field_dict["redirect_uri"] = redirect_uri

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.provider_settings import ProviderSettings # noqa: PLC0415
        d = dict(src_dict)
        id = d.pop("id")

        tenant_id = d.pop("tenant_id")

        name = d.pop("name")

        kind = d.pop("kind")

        enabled = d.pop("enabled")

        settings = ProviderSettings.from_dict(d.pop("settings"))




        has_client_secret = d.pop("has_client_secret")

        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




        def _parse_redirect_uri(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        redirect_uri = _parse_redirect_uri(d.pop("redirect_uri", UNSET))


        identity_provider_response = cls(
            id=id,
            tenant_id=tenant_id,
            name=name,
            kind=kind,
            enabled=enabled,
            settings=settings,
            has_client_secret=has_client_secret,
            created_at=created_at,
            updated_at=updated_at,
            redirect_uri=redirect_uri,
        )


        identity_provider_response.additional_properties = d
        return identity_provider_response

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
