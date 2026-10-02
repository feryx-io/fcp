from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.discovered_provider import DiscoveredProvider





T = TypeVar("T", bound="DiscoverResponse")



@_attrs_define
class DiscoverResponse:
    """ Where a login signs in, and how.

        Attributes:
            methods (list[str]): The ways in: `password`, and `sso` when the tenant has identity
                providers to sign in with.
            tenant_id (None | str | Unset): The tenant the login's email domain belongs to, when it is one.
            tenant_name (None | str | Unset):
            providers (list[DiscoveredProvider] | Unset): The tenant's identity providers: a button each on the login page,
                which goes to `/api/v1/auth/sso/{id}/start`.
     """

    methods: list[str]
    tenant_id: None | str | Unset = UNSET
    tenant_name: None | str | Unset = UNSET
    providers: list[DiscoveredProvider] | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.discovered_provider import DiscoveredProvider # noqa: PLC0415
        methods = self.methods



        tenant_id: None | str | Unset
        if isinstance(self.tenant_id, Unset):
            tenant_id = UNSET
        else:
            tenant_id = self.tenant_id

        tenant_name: None | str | Unset
        if isinstance(self.tenant_name, Unset):
            tenant_name = UNSET
        else:
            tenant_name = self.tenant_name

        providers: list[dict[str, Any]] | Unset = UNSET
        if not isinstance(self.providers, Unset):
            providers = []
            for providers_item_data in self.providers:
                providers_item = providers_item_data.to_dict()
                providers.append(providers_item)




        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "methods": methods,
        })
        if tenant_id is not UNSET:
            field_dict["tenant_id"] = tenant_id
        if tenant_name is not UNSET:
            field_dict["tenant_name"] = tenant_name
        if providers is not UNSET:
            field_dict["providers"] = providers

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.discovered_provider import DiscoveredProvider # noqa: PLC0415
        d = dict(src_dict)
        methods = cast(list[str], d.pop("methods"))


        def _parse_tenant_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        tenant_id = _parse_tenant_id(d.pop("tenant_id", UNSET))


        def _parse_tenant_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        tenant_name = _parse_tenant_name(d.pop("tenant_name", UNSET))


        _providers = d.pop("providers", UNSET)
        providers: list[DiscoveredProvider] | Unset = UNSET
        if _providers is not UNSET:
            providers = []
            for providers_item_data in _providers:
                providers_item = DiscoveredProvider.from_dict(providers_item_data)



                providers.append(providers_item)


        discover_response = cls(
            methods=methods,
            tenant_id=tenant_id,
            tenant_name=tenant_name,
            providers=providers,
        )


        discover_response.additional_properties = d
        return discover_response

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
